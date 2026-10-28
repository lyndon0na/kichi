//! 下载流：把带签名的直链变成可 `Read` 的字节流，支持 HTTP Range 续传。
//!
//! 从 `media.rs` 拆出来守行数上限：那边只负责详情与选链，这里负责
//! 「怎么把字节拉下来」。两条纪律：
//! - 下载必须用**单独的** reqwest client，`ClientBuilder::timeout(idle)` 的语义是
//!   「等响应头 ≤ idle + 每次 read ≤ idle」，没有整文件总时限；**不能**给 request
//!   加 `.timeout()`，那会变成总时限（见 `docs/pitfalls.md` 第 14 条）。
//! - 直链是带签名的临时 URL，不缓存、不打印（错误里最多带状态码）。
//!
//! Range 语义（2026-10-27 新增）：
//! | 响应 | 含义 | 处理 |
//! | --- | --- | --- |
//! | 206 | 接受 Range | 解析 `Content-Range`；`start` 必须等于请求偏移 |
//! | 200 | 服务器忽略 Range | `start_offset = 0`，调用方截断 part 从头写（有界分片请求直接报错回退） |
//! | 416 | 偏移越界 | 返回 [`Error::RangeNotSatisfiable`]，调用方截断重来 |
//! | 206 缺 `Content-Range` | 无法确认断点 | 无界请求改发不带 Range 的请求；有界分片请求报错回退 |

use std::io;
use std::sync::Arc;
use std::time::Duration;

use reqwest::header::{CONTENT_RANGE, RANGE};
use reqwest::StatusCode;

use crate::drive::DriveClient;
use crate::media::FileInfo;
use crate::Error;

/// 下载空闲读超时：连续这么久没收到任何字节就判失败。
///
/// 必须建在**下载 client** 上（响应头 + 每次读各算一次），
/// 不能用 request 级 `.timeout()`——那会变成整个文件的总时限。
pub const DEFAULT_DOWNLOAD_IDLE: Duration = Duration::from_secs(30);

/// 覆盖空闲读超时的环境变量（秒）。
pub const DOWNLOAD_IDLE_ENV: &str = "KICHI_DOWNLOAD_IDLE_TIMEOUT";

/// 当前生效的空闲读超时。
///
/// 空串、非法数字、`0` 一律回默认值：0 会让每次读立即超时，不可能是本意。
pub fn download_idle() -> Duration {
    std::env::var(DOWNLOAD_IDLE_ENV)
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_DOWNLOAD_IDLE)
}

impl DriveClient {
    /// 打开一个文件的下载流（已跟随重定向，`Read` 即文件字节）。
    ///
    /// 直链是带签名的临时 URL，**只发这一次 GET**，不复用 [`crate::http::HttpClient`]
    /// ——那边的 30 秒总超时会把大文件拦腰砍断；这里的超时建在下载 client 上：
    /// 等响应头最多 [`download_idle`]，之后每次读最多 [`download_idle`]，
    /// 没有整个文件的总时限。
    ///
    /// 空文件与「官方没有下发链接」都会在真正发请求前就返回错误，省一次往返。
    pub fn open_download(&self, info: &FileInfo) -> Result<DownloadStream, Error> {
        self.open_download_from(info, 0)
    }

    /// 从 `offset` 处打开下载流。`offset == 0` 与 [`DriveClient::open_download`] 完全一致。
    ///
    /// `offset > 0` 时发 `Range: bytes=<offset>-`：
    /// - 206：`start_offset == offset`，`total_bytes` 是完整文件大小；
    /// - 200：服务器忽略了 Range，`start_offset == 0`，调用方必须截断 part；
    /// - 416：返回 [`Error::RangeNotSatisfiable`]。
    pub fn open_download_from(
        &self,
        info: &FileInfo,
        offset: u64,
    ) -> Result<DownloadStream, Error> {
        self.open_download_from_with(info, offset, download_idle())
    }

    /// 同 [`DriveClient::open_download_from`]，但可指定空闲读超时（测试注入短超时用）。
    pub fn open_download_from_with(
        &self,
        info: &FileInfo,
        offset: u64,
        idle: Duration,
    ) -> Result<DownloadStream, Error> {
        self.open_range(info, offset, None, idle)
    }

    /// 打开一个**有界**区间 `[start, end]`（闭区间，对应 `Range: bytes=start-end`）。
    ///
    /// 供单文件多分片使用。校验与 [`DriveClient::open_download_from`] 一致：
    /// `start` 必须等于请求起点，完整大小取 `Content-Range` 的 total。
    ///
    /// 服务器返回 200（忽略 Range）或 206 缺 `Content-Range` 时**不能**把这批
    /// 字节写进分片区间，返回 [`Error::RangeNotSatisfiable`] 让会话层退回单连接。
    pub fn open_download_range(
        &self,
        info: &FileInfo,
        start: u64,
        end: u64,
    ) -> Result<DownloadStream, Error> {
        self.open_download_range_with(info, start, end, download_idle())
    }

    /// 同 [`DriveClient::open_download_range`]，但可指定空闲读超时（测试注入用）。
    pub fn open_download_range_with(
        &self,
        info: &FileInfo,
        start: u64,
        end: u64,
        idle: Duration,
    ) -> Result<DownloadStream, Error> {
        if start > end {
            return Err(Error::InvalidInput(format!(
                "分片区间不合法：{start}-{end}"
            )));
        }
        self.open_range(info, start, Some(end), idle)
    }

    /// `end` 为 `None` 时是「从偏移到文件末尾」，`Some` 时是闭区间分片。
    fn open_range(
        &self,
        info: &FileInfo,
        start: u64,
        end: Option<u64>,
        idle: Duration,
    ) -> Result<DownloadStream, Error> {
        let Some(url) = info.download_url() else {
            return Err(Error::Other(
                "官方没有下发这个文件的下载链接（可能是违规或受限文件）".to_owned(),
            ));
        };
        // 先取流再写盘：这样「下载 0 字节就成功」不会变成一个没有内容的空文件
        let client = self.download_client(idle)?;
        let mut request = client.get(&url);
        match end {
            Some(end) => request = request.header(RANGE, format!("bytes={start}-{end}")),
            None if start > 0 => request = request.header(RANGE, format!("bytes={start}-")),
            None => {}
        }
        let response = request.send().map_err(|e| {
            // 连接阶段先失败时说「连不上」，别说成「服务器没响应」
            if e.is_timeout() && !e.is_connect() {
                Error::DownloadTimeout(format!(
                    "下载超时：{} 秒内没有收到下载服务器的响应",
                    idle.as_secs()
                ))
            } else {
                Error::Network(network_reason(&e))
            }
        })?;
        let status = response.status();

        if status == StatusCode::RANGE_NOT_SATISFIABLE {
            let total = response
                .headers()
                .get(CONTENT_RANGE)
                .and_then(|value| value.to_str().ok())
                .and_then(parse_content_range)
                .and_then(|range| range.total);
            return Err(Error::RangeNotSatisfiable { total });
        }
        if !status.is_success() {
            // CDN 的错误页不是官方 JSON；带上状态码即可，别把整页 HTML 塞进界面
            return Err(Error::Api {
                code: i64::from(status.as_u16()),
                reason: format!("download_http_{}", status.as_u16()),
                message: "下载服务器拒绝了这个请求（链接可能已过期，可重试一次）".to_owned(),
            });
        }

        if status == StatusCode::PARTIAL_CONTENT {
            let range = response
                .headers()
                .get(CONTENT_RANGE)
                .and_then(|value| value.to_str().ok())
                .and_then(parse_content_range);
            let Some(range) = range else {
                // 分片区间无法确认起点 / 总大小时绝不能猜：退回单连接重下
                if end.is_some() {
                    return Err(Error::RangeNotSatisfiable { total: None });
                }
                if start == 0 {
                    // 从 0 开始的 206 即使没有 Content-Range，Content-Length 也是完整长度
                    let total = response.content_length();
                    return Ok(DownloadStream {
                        response,
                        total_bytes: total,
                        idle,
                        start_offset: 0,
                    });
                }
                // 无法确认断点位置：丢掉这条响应，改发不带 Range 的请求（安全优先）
                drop(response);
                return self.open_range(info, 0, None, idle);
            };
            let Some(received_start) = range.start else {
                return Err(Error::RangeNotSatisfiable { total: range.total });
            };
            if received_start != start {
                return Err(Error::RangeNotSatisfiable { total: range.total });
            }
            // 206 的 Content-Length 只是剩余字节；完整大小以 Content-Range 为准。
            // 分片响应里 `range.end` 只是这一片的末尾，不能当文件总大小。
            let total = if end.is_some() {
                range.total.or(info.size)
            } else {
                range
                    .total
                    .or(info.size)
                    .or_else(|| range.end.map(|end| end.saturating_add(1)))
            };
            return Ok(DownloadStream {
                response,
                total_bytes: total,
                idle,
                start_offset: received_start,
            });
        }

        // 200：服务器忽略了 Range。分片请求不能把整文件写进一片，交给会话层退回
        // 单连接重下；不带分片的 0 偏移请求仍按完整文件处理。
        if end.is_some() {
            return Err(Error::RangeNotSatisfiable {
                total: response.content_length(),
            });
        }
        let total = response.content_length();
        Ok(DownloadStream {
            response,
            total_bytes: total,
            idle,
            start_offset: 0,
        })
    }

    /// 同 [`DriveClient::open_download_from_with`]，但不发送 Range（兼容旧调用与测试）。
    pub fn open_download_with(
        &self,
        info: &FileInfo,
        idle: Duration,
    ) -> Result<DownloadStream, Error> {
        self.open_download_from_with(info, 0, idle)
    }

    /// 下载专用 client（每个 `DriveClient` 只建一个，按空闲读超时缓存）。
    ///
    /// **必须**用 client 级 `.timeout()`：blocking 版的语义是「等响应头 ≤ D +
    /// 每次 `read` ≤ D」，没有整体时长上限。request 级 `.timeout()` 会被写进
    /// async 层变成总时限，大文件会被拦腰砍断（见 `docs/pitfalls.md`）。
    ///
    /// 缓存键是 `idle`：正常路径永远是 [`download_idle`]，只有测试注入短超时
    /// （以及同进程内改 `KICHI_DOWNLOAD_IDLE_TIMEOUT`）才会换一把。
    fn download_client(&self, idle: Duration) -> Result<Arc<reqwest::blocking::Client>, Error> {
        {
            let cached = self.download.lock().unwrap_or_else(|e| e.into_inner());
            if let Some((cached_idle, client)) = cached.as_ref() {
                if *cached_idle == idle {
                    return Ok(Arc::clone(client));
                }
            }
        }
        // 锁外构建：build 会起一条专属运行时线程，不在锁里做
        let built = Arc::new(
            reqwest::blocking::Client::builder()
                .user_agent(&self.config().user_agent)
                .connect_timeout(Duration::from_secs(20))
                .timeout(idle)
                .build()
                .map_err(|e| Error::Network(format!("无法创建下载客户端：{e}")))?,
        );
        let mut cached = self.download.lock().unwrap_or_else(|e| e.into_inner());
        *cached = Some((idle, Arc::clone(&built)));
        Ok(built)
    }
}

/// 正在下载的字节流。
///
/// `total_bytes` 是**完整文件大小**（不是剩余字节）；`start_offset` 是这条响应
/// 实际从文件的哪个位置开始：`0` = 服务器忽略了 Range 或本来就是从头下载。
pub struct DownloadStream {
    response: reqwest::blocking::Response,
    total_bytes: Option<u64>,
    /// 每次读最多等这么久（连续没收到字节就判失败）
    idle: Duration,
    start_offset: u64,
}

impl DownloadStream {
    /// 官方声明的总大小（可能为 `None`）。
    pub fn total_bytes(&self) -> Option<u64> {
        self.total_bytes
    }

    /// 这条响应实际从文件的哪个位置开始。
    pub fn start_offset(&self) -> u64 {
        self.start_offset
    }
}

impl std::io::Read for DownloadStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self.response.read(buf) {
            // blocking 的 Response 把等待超时包成 io::ErrorKind::Other + reqwest::Error；
            // 这里翻成标准 TimedOut，session 层据此给「多久没收到数据」的文案。
            Err(err) if is_timeout(&err) => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("{} 秒没有收到数据", self.idle.as_secs()),
            )),
            other => other,
        }
    }
}

/// 解析出的 `Content-Range`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ContentRange {
    /// `*`（416 的 `bytes */total`）或无法解析时为 `None`
    start: Option<u64>,
    end: Option<u64>,
    total: Option<u64>,
}

/// 解析 `bytes <start>-<end>/<total>` 或 `bytes */<total>`。
///
/// 只接受 `bytes` 单位；`total` 为 `*` 时返回 `None`。任何格式问题都返回 `None`，
/// 由调用方决定是重来还是报错。
fn parse_content_range(value: &str) -> Option<ContentRange> {
    let value = value.trim();
    let (unit, rest) = value.split_once(' ')?;
    if !unit.eq_ignore_ascii_case("bytes") {
        return None;
    }
    let (range, total) = rest.split_once('/')?;
    let total = match total.trim() {
        "*" => None,
        text => Some(text.parse::<u64>().ok()?),
    };
    if range.trim() == "*" {
        return Some(ContentRange {
            start: None,
            end: None,
            total,
        });
    }
    let (start, end) = range.split_once('-')?;
    let start = start.trim().parse::<u64>().ok()?;
    let end = end.trim().parse::<u64>().ok()?;
    if end < start {
        return None;
    }
    Some(ContentRange {
        start: Some(start),
        end: Some(end),
        total,
    })
}

/// 这个 io 错误是不是等待超时（reqwest 的包装或标准 TimedOut）。
fn is_timeout(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::TimedOut
        || err
            .get_ref()
            .and_then(|source| source.downcast_ref::<reqwest::Error>())
            .is_some_and(reqwest::Error::is_timeout)
}

/// 网络错误文本（不含凭据；直链本身绝不打印）。
fn network_reason(err: &reqwest::Error) -> String {
    if err.is_connect() {
        "无法连接下载服务器".to_owned()
    } else if err.is_timeout() {
        "下载超时".to_owned()
    } else {
        format!("下载中断：{err}")
    }
}

#[cfg(test)]
#[path = "dl_tests.rs"]
mod tests;
