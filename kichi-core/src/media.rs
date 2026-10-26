//! 播放：文件详情与画质档位选择。
//!
//! 选链规则借用 pikpaktui（Apache-2.0）的行为结论，代码自己写：
//! - `medias[]` 是转码档列表，`is_origin` 为原画；隐藏（`is_visible: false`）
//!   与需要额外配额（`need_more_quota: true`）的档位一律不可用；
//! - **不得绕过限制**：medias 存在但全部不可用时，不能回退到下载直链，
//!   否则等于用下载链接绕过官方的播放权限；
//! - 只有 medias 缺失或为空（非视频文件、老响应）时，才用
//!   `web_content_link` / `links["application/octet-stream"]` 兜底为「原画」。
//!
//! 直链是带签名的临时 URL：不缓存、不打印（最多打到域名）。

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::drive::DriveClient;
use crate::types::{PlayOption, StreamKind, Tier};
use crate::Error;

/// 档位不可用原因（界面层负责翻成中文）。
pub const REASON_HIDDEN: &str = "hidden";
pub const REASON_QUOTA: &str = "quota";
pub const REASON_NOT_READY: &str = "not_ready";

/// 文件详情里与播放相关的部分。
#[derive(Debug, Clone, Default)]
pub struct FileInfo {
    pub id: String,
    pub name: String,
    /// 原始下载直链（可能为空：非视频或官方未下发）
    pub web_content_link: String,
    /// `links["application/octet-stream"].url`
    pub octet_stream: String,
    /// `medias` 原始档位
    pub medias: Vec<MediaStream>,
    /// 该文件是否没有任何可用链接（session 层据此延迟重试）
    pub all_links_empty: bool,
}

/// `medias[]` 的一项。
#[derive(Debug, Clone, Default)]
pub struct MediaStream {
    pub name: String,
    pub url: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub original: bool,
    pub visible: bool,
    pub need_more_quota: bool,
    pub height: Option<u32>,
}

impl FileInfo {
    /// 选出可播放的档位（原画在前，转码按清晰度降序）。
    pub fn play_options(&self) -> Vec<PlayOption> {
        let mut options: Vec<PlayOption> = self.medias.iter().map(MediaStream::to_option).collect();
        // 排序：原画优先，其余按高度降序（未知高度排后面）
        options.sort_by_key(|option| {
            (
                !option.original,
                std::cmp::Reverse(option.height.unwrap_or(0)),
            )
        });

        if options.is_empty() {
            if let Some(url) = self.fallback_url() {
                options.push(PlayOption {
                    label: Tier::Original.label().to_owned(),
                    url,
                    kind: StreamKind::Original,
                    height: None,
                    original: true,
                    tier: Some(Tier::Original),
                    available: true,
                    reason: None,
                });
            }
        }
        options
    }

    /// 下载用直链。
    ///
    /// 顺序照 rclone（`setMetaData`）：
    /// 1. `links["application/octet-stream"]`；
    /// 2. 若某档位链接 URL 的 `fid` 与它相同（= 原画那条），换成该档位链接——
    ///    官方给 media 链接分配了不同的 CDN，速度通常更好；
    /// 3. 没有 octet-stream 时退回 `web_content_link`。
    ///
    /// 注意：**不是** [`FileInfo::play_options`]，与播放权限无关；
    /// 这里关心的是「能不能把整个文件拉回本地」。
    pub fn download_url(&self) -> Option<String> {
        let octet = self.octet_stream.trim();
        if !octet.is_empty() {
            let target_fid = query_param(octet, "fid");
            if let Some(fid) = target_fid {
                if let Some(media) = self.medias.iter().find(|media| {
                    !media.url.is_empty()
                        && query_param(&media.url, "fid").as_deref() == Some(fid.as_str())
                }) {
                    return Some(media.url.clone());
                }
            }
            return Some(octet.to_owned());
        }
        let web = self.web_content_link.trim();
        (!web.is_empty()).then(|| web.to_owned())
    }

    /// 兜底下载直链（仅在没有任何 medias 时使用）。
    fn fallback_url(&self) -> Option<String> {
        [&self.web_content_link, &self.octet_stream]
            .into_iter()
            .map(|url| url.trim())
            .find(|url| !url.is_empty())
            .map(str::to_owned)
    }

    /// 还有没有档位能播。
    pub fn has_playable(&self) -> bool {
        self.play_options().iter().any(PlayOption::playable)
    }
}

impl MediaStream {
    fn to_option(&self) -> PlayOption {
        let reason = if self.need_more_quota {
            Some(REASON_QUOTA)
        } else if !self.visible {
            Some(REASON_HIDDEN)
        } else if self.url.trim().is_empty() {
            Some(REASON_NOT_READY)
        } else {
            None
        };
        let (label, kind, tier) = self.label_kind_and_tier();
        PlayOption {
            label,
            url: self.url.clone(),
            kind,
            height: self.height,
            original: self.original,
            tier,
            available: reason.is_none(),
            reason,
        }
    }

    /// 显示名 + 链接类别（转码档走 m3u8，原画走文件直链）+ 归到哪个可选档位。
    fn label_kind_and_tier(&self) -> (String, StreamKind, Option<Tier>) {
        if self.original {
            return (
                Tier::Original.label().to_owned(),
                StreamKind::Original,
                Some(Tier::Original),
            );
        }
        let label = if self.name.trim().is_empty() {
            "转码".to_owned()
        } else {
            self.name.trim().to_owned()
        };
        let tier = Tier::from_stream_name(&label);
        (label, StreamKind::Media, tier)
    }
}

impl DriveClient {
    /// 取一个文件的详情（直链 + medias 列表）。
    pub fn file_detail(&self, id: &str) -> Result<FileInfo, Error> {
        let id = id.trim();
        if id.is_empty() {
            return Err(Error::InvalidInput("文件 id 为空".into()));
        }
        let dto: FileDto = self.get(&self.config().file_url(id))?;
        Ok(dto.into_info())
    }

    /// 打开一个文件的下载流（已跟随重定向，`Read` 即文件字节）。
    ///
    /// 直链是带签名的临时 URL，**只发这一次 GET**，不复用 [`crate::http::HttpClient`]
    /// ——那边的 30 秒总超时会把大文件拦腰砍断；这里连接超时短、读超时由调用方
    /// 在逐块读取时自行计。
    ///
    /// 空文件与「官方没有下发链接」都会在真正发请求前就返回错误，省一次往返。
    pub fn open_download(&self, info: &FileInfo) -> Result<DownloadStream, Error> {
        let Some(url) = info.download_url() else {
            return Err(Error::Other(
                "官方没有下发这个文件的下载链接（可能是违规或受限文件）".to_owned(),
            ));
        };
        // 先取流再写盘：这样「下载 0 字节就成功」不会变成一个没有内容的空文件
        let client = reqwest::blocking::Client::builder()
            .user_agent(&self.config().user_agent)
            .connect_timeout(Duration::from_secs(20))
            .build()
            .map_err(|e| Error::Network(format!("无法创建下载客户端：{e}")))?;
        let response = client
            .get(&url)
            .send()
            .map_err(|e| Error::Network(network_reason(&e)))?;
        let status = response.status();
        if !status.is_success() {
            // CDN 的错误页不是官方 JSON；带上状态码即可，别把整页 HTML 塞进界面
            return Err(Error::Api {
                code: i64::from(status.as_u16()),
                reason: format!("download_http_{}", status.as_u16()),
                message: "下载服务器拒绝了这个请求（链接可能已过期，可重试一次）".to_owned(),
            });
        }
        let total = response.content_length();
        Ok(DownloadStream {
            response,
            total_bytes: total,
        })
    }
}

/// 正在下载的字节流。`total_bytes` 来自 `Content-Length`（CDN 不给时为 `None`）。
pub struct DownloadStream {
    response: reqwest::blocking::Response,
    total_bytes: Option<u64>,
}

impl DownloadStream {
    /// 官方声明的总大小（可能为 `None`）。
    pub fn total_bytes(&self) -> Option<u64> {
        self.total_bytes
    }
}

impl std::io::Read for DownloadStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.response.read(buf)
    }
}

/// 网络错误文本（不含凭据；直链本身绝不打印）。
fn network_reason(err: &reqwest::Error) -> String {
    if err.is_timeout() {
        "下载超时".to_owned()
    } else if err.is_connect() {
        "无法连接下载服务器".to_owned()
    } else {
        format!("下载中断：{err}")
    }
}

#[derive(Deserialize)]
struct FileDto {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    web_content_link: String,
    #[serde(default)]
    links: Option<LinksDto>,
    #[serde(default)]
    medias: Vec<MediaDto>,
}

#[derive(Deserialize)]
struct LinksDto {
    #[serde(default, rename = "application/octet-stream")]
    octet_stream: Option<LinkDto>,
}

#[derive(Deserialize)]
struct LinkDto {
    #[serde(default)]
    url: String,
    #[serde(default)]
    expire: Option<String>,
}

#[derive(Deserialize)]
struct MediaDto {
    #[serde(default)]
    media_name: String,
    #[serde(default)]
    is_origin: bool,
    #[serde(default = "default_visible")]
    is_visible: bool,
    #[serde(default)]
    need_more_quota: bool,
    #[serde(default)]
    link: Option<LinkDto>,
    #[serde(default)]
    video: Option<VideoDto>,
}

fn default_visible() -> bool {
    true
}

#[derive(Deserialize)]
struct VideoDto {
    #[serde(default)]
    height: Option<u32>,
}

impl FileDto {
    fn into_info(self) -> FileInfo {
        let octet = self
            .links
            .and_then(|links| links.octet_stream)
            .map(|link| link.url)
            .unwrap_or_default();
        let medias = self
            .medias
            .into_iter()
            .map(|media| {
                let url = media
                    .link
                    .as_ref()
                    .map(|link| link.url.trim().to_owned())
                    .unwrap_or_default();
                let expires_at = media
                    .link
                    .as_ref()
                    .and_then(|link| link.expire.as_deref())
                    .and_then(parse_time);
                MediaStream {
                    name: media.media_name.clone(),
                    url,
                    expires_at,
                    original: media.is_origin,
                    visible: media.is_visible,
                    need_more_quota: media.need_more_quota,
                    height: media
                        .video
                        .and_then(|video| video.height)
                        .filter(|h| *h > 0)
                        .or_else(|| height_from_label(&media.media_name)),
                }
            })
            .collect::<Vec<_>>();
        let all_links_empty = medias.iter().all(|media| media.url.is_empty())
            && self.web_content_link.trim().is_empty()
            && octet.trim().is_empty();
        FileInfo {
            id: self.id,
            name: self.name,
            web_content_link: self.web_content_link,
            octet_stream: octet,
            medias,
            all_links_empty,
        }
    }
}

/// 从 URL 查询串里取一个参数（`fid` 用）。
///
/// 不引入 url crate：只按 `&` 拆、按 `=` 切，值做最小百分号解码。
fn query_param(url: &str, key: &str) -> Option<String> {
    let query = url.split_once('?').map(|(_, rest)| rest)?;
    let query = query.split('#').next().unwrap_or(query);
    for pair in query.split('&') {
        let (name, value) = pair.split_once('=')?;
        if name == key {
            return Some(percent_decode(value));
        }
    }
    None
}

/// 最小百分号解码（只处理 `%XX` 与 `+`；其余原样保留）。
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        out.push(byte);
                        index += 3;
                    }
                    Err(_) => {
                        out.push(b'%');
                        index += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// RFC3339 时间；空串 / 解析失败一律当作「没有」。
fn parse_time(text: &str) -> Option<DateTime<Utc>> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|time| time.with_timezone(&Utc))
}

/// 从档位名解析清晰度：`"1080P"` → 1080，`"4K"` → 2160。
///
/// 官方一般会给 `video.height`，这里只做兜底（解析不出返回 None）。
fn height_from_label(name: &str) -> Option<u32> {
    let label = name.trim().to_ascii_lowercase();
    if label == "4k" {
        return Some(2160);
    }
    let digits: String = label.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse::<u32>().ok().filter(|h| *h > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 解码官方结构的示例响应（字段形状取自 rclone 的 api.File）。
    fn info(json: &str) -> FileInfo {
        let dto: FileDto = serde_json::from_str(json).unwrap_or_else(|err| panic!("{err}"));
        dto.into_info()
    }

    const MIXED: &str = r#"{
        "id": "f1", "name": "movie.mkv",
        "web_content_link": "https://download.invalid/slow",
        "links": {"application/octet-stream": {"url": "https://download.invalid/octet"}},
        "medias": [
            {"media_name": "720P", "is_visible": true, "link": {"url": "https://media.invalid/720"}},
            {"media_name": "1080P", "is_origin": true, "is_visible": true,
             "link": {"url": "https://media.invalid/original?expire=1", "expire": "2030-01-01T00:00:00+08:00"}},
            {"media_name": "Hidden", "is_visible": false, "link": {"url": "https://media.invalid/hidden"}},
            {"media_name": "2160P", "is_visible": true, "need_more_quota": true, "link": {"url": "https://media.invalid/2160"}},
            {"media_name": "480P", "is_visible": true, "link": {"url": ""}}
        ]
    }"#;

    #[test]
    fn original_comes_first_then_transcodes_by_height() {
        let info = info(MIXED);
        let options = info.play_options();
        assert_eq!(options.len(), 5);
        assert_eq!(options[0].label, "原画");
        assert!(options[0].original);
        assert_eq!(options[0].kind, StreamKind::Original);
        // 剩余按高度降序：2160P(不可用) → 720P → 480P(未就绪) → Hidden
        assert_eq!(
            options[1..]
                .iter()
                .map(|o| o.label.as_str())
                .collect::<Vec<_>>(),
            vec!["2160P", "720P", "480P", "Hidden"]
        );
        assert_eq!(options[1].kind, StreamKind::Media, "转码档按 m3u8 处理");
        assert_eq!(options[1].reason, Some(REASON_QUOTA));
        assert!(!options[1].available);
        assert!(options[2].playable());
        assert_eq!(options[3].reason, Some(REASON_NOT_READY));
        assert_eq!(options[4].reason, Some(REASON_HIDDEN));
    }

    #[test]
    fn restricted_media_never_falls_back_to_download_link() {
        // 原画被隐藏：官方不允许播放，也不许拿下载链接绕过
        let info = info(
            r#"{"name": "x", "web_content_link": "https://download.invalid/file",
                "medias": [{"is_origin": true, "is_visible": false, "link": {"url": "https://media.invalid/original"}}]}"#,
        );
        let options = info.play_options();
        assert_eq!(options.len(), 1);
        assert!(!options[0].available);
        assert!(
            options[0].url.starts_with("https://media.invalid"),
            "不得替换成下载直链"
        );
        assert!(!info.has_playable());
    }

    #[test]
    fn options_are_grouped_into_the_fixed_tiers() {
        let options = info(MIXED).play_options();
        let tiers: Vec<_> = options.iter().map(|o| o.tier).collect();
        assert_eq!(
            tiers,
            vec![
                Some(Tier::Original),
                None, // 2160P：菜单里不出现
                Some(Tier::Transcode("720P")),
                Some(Tier::Transcode("480P")),
                None, // Hidden
            ]
        );
    }

    #[test]
    fn legacy_response_without_medias_uses_download_link() {
        let info =
            info(r#"{"name": "a.mp3", "web_content_link": "https://download.invalid/file"}"#);
        let options = info.play_options();
        assert_eq!(options.len(), 1);
        assert!(options[0].original && options[0].playable());
        assert_eq!(options[0].url, "https://download.invalid/file");
    }

    #[test]
    fn empty_web_content_link_falls_through_to_octet_stream() {
        let info = info(
            r#"{"name": "a.mp3", "web_content_link": "  ",
                "links": {"application/octet-stream": {"url": "https://download.invalid/fallback"}}}"#,
        );
        assert_eq!(
            info.play_options()[0].url,
            "https://download.invalid/fallback"
        );
    }

    #[test]
    fn height_parses_from_metadata_only_and_label_is_readable() {
        // 官方给的高度优先
        let with_meta = info(
            r#"{"medias": [{"media_name": "2160P", "video": {"height": 1440}, "link": {"url": "u1"}}]}"#,
        );
        assert_eq!(with_meta.play_options()[0].height, Some(1440));
        // 缺字段时从档位名兜底
        let from_label = info(
            r#"{"medias": [
                {"media_name": "1080P", "link": {"url": "u1"}},
                {"media_name": "4K", "link": {"url": "u2"}},
                {"media_name": "", "link": {"url": "u3"}}
            ]}"#,
        );
        let options = from_label.play_options();
        assert_eq!(
            options
                .iter()
                .map(|o| (o.label.as_str(), o.height))
                .collect::<Vec<_>>(),
            vec![("4K", Some(2160)), ("1080P", Some(1080)), ("转码", None)]
        );
        // 没有名字也不至于让界面空着
        let unnamed = info(r#"{"medias": [{"link": {"url": "u"}}]}"#);
        assert_eq!(unnamed.play_options()[0].label, "转码");
        assert!(unnamed.play_options()[0].playable());
    }

    #[test]
    fn download_url_prefers_media_link_with_same_fid() {
        // octet-stream 与某档位共享同一个 fid → 用那条 media 链接（rclone 行为）
        let info = info(
            r#"{
                "name": "movie.mkv",
                "web_content_link": "https://download.invalid/web",
                "links": {"application/octet-stream": {"url": "https://download.invalid/o?fid=F1&expire=1"}},
                "medias": [
                    {"media_name": "720P", "link": {"url": "https://media.invalid/720?fid=F9"}},
                    {"media_name": "原画", "is_origin": true, "link": {"url": "https://media.invalid/orig?fid=F1&x=2"}}
                ]
            }"#,
        );
        assert_eq!(
            info.download_url().as_deref(),
            Some("https://media.invalid/orig?fid=F1&x=2")
        );
    }

    #[test]
    fn download_url_without_matching_media_uses_octet_stream() {
        let info = info(
            r#"{"name": "a.bin",
                "web_content_link": "https://download.invalid/web",
                "links": {"application/octet-stream": {"url": "https://download.invalid/o?fid=F1"}},
                "medias": [{"link": {"url": "https://media.invalid/other?fid=F2"}}]}"#,
        );
        assert_eq!(
            info.download_url().as_deref(),
            Some("https://download.invalid/o?fid=F1")
        );
    }

    #[test]
    fn download_url_falls_back_to_web_content_link() {
        // 没有 octet-stream：用 web_content_link；都没有就是 None
        let web = info(r#"{"web_content_link": "https://download.invalid/web"}"#);
        assert_eq!(
            web.download_url().as_deref(),
            Some("https://download.invalid/web")
        );
        let none = info(r#"{"name": "x"}"#);
        assert_eq!(none.download_url(), None);
        let blank = info(r#"{"web_content_link": "   "}"#);
        assert_eq!(blank.download_url(), None);
    }

    #[test]
    fn query_param_decodes_and_misses_cleanly() {
        assert_eq!(
            query_param("https://x.invalid/a?fid=AB%2BC&t=1", "fid").as_deref(),
            Some("AB+C")
        );
        assert_eq!(query_param("https://x.invalid/a?t=1", "fid"), None);
        assert_eq!(query_param("https://x.invalid/a", "fid"), None);
    }

    #[test]
    fn all_links_empty_is_reported_for_retry() {
        let empty = info(r#"{"name": "x", "medias": [{"link": {"url": "  "}}]}"#);
        assert!(empty.all_links_empty);
        let ready = info(MIXED);
        assert!(!ready.all_links_empty);
    }

    #[test]
    fn expire_query_wins_over_link_expire() {
        use chrono::Duration;
        let past = Utc::now() - Duration::hours(1);
        let mut option = PlayOption {
            label: "原画".into(),
            url: "https://cdn.invalid/v?expire=1&x=2".into(),
            kind: StreamKind::Original,
            height: None,
            original: true,
            tier: Some(Tier::Original),
            available: true,
            reason: None,
        };
        // URL 里的 expire=1（1970 年）已过期，即便 link.expire 在未来
        assert!(option.is_expired(Some(Utc::now() + Duration::hours(1))));
        let future = (Utc::now() + Duration::hours(1)).timestamp();
        option.url = format!("https://cdn.invalid/v?a=1&expire={future}");
        assert!(!option.is_expired(Some(past)));
        // 没有 expire 参数时退回 link.expire
        option.url = "https://cdn.invalid/v".into();
        assert!(option.is_expired(Some(past)));
        assert!(!option.is_expired(None), "时间未知时不当作过期");
    }
}
