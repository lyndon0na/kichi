//! 断点续传的执行细节：Part / 选断点 / 流式追加 / 重试决策。
//!
//! 从 `download.rs` 拆出来守行数上限：那边管「一次下载」的入口与取消语义，
//! 这里管「从哪个字节继续、写到哪里、失败了怎么重试」。纯函数（`plan_resume`）
//! 与索引在 `partials.rs`，HTTP Range 语义在 `kichi-core/src/dl.rs`。
//!
//! 中止分流见 [`Part::abort`]：用户取消删 part；登出（abandon）保留 part +
//! 索引，且任何「已下完只差 rename」的分支都要在提交锁内再查一次登出。

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use kichi_core::Error as CoreError;

use super::download::{commit_lock, Cancel, Downloaded, Progress};
use super::download_naming::{claim_part, finalize_part};
use super::partials::{self, plan_resume, PartialEntry};
use super::Failure;
use crate::format::size as readable_size;

/// 每次读的块大小（64 KiB：够大够快，进度回调也不至于太密）。
pub(super) const CHUNK: usize = 64 * 1024;
/// 断流自动重试次数（默认；`KICHI_DOWNLOAD_RETRIES` 可覆盖，0 = 关闭）。
pub(super) const DEFAULT_RETRIES: u32 = 3;
/// 重试等待：1s / 3s / 8s；超过三次的配置沿用 8s。
pub(super) const RETRY_DELAYS: [Duration; 3] = [
    Duration::from_secs(1),
    Duration::from_secs(3),
    Duration::from_secs(8),
];
/// 覆盖自动重试次数的环境变量。
pub(super) const RETRIES_ENV: &str = "KICHI_DOWNLOAD_RETRIES";
/// 整次下载的重试总上限 = 连续上限的这么多倍。
///
/// 连续预算（`DEFAULT_RETRIES`）管「一波抖动别拖死」；这条管「服务器每次只给
/// 几 KB 就断开」——每次都有进展、连续计数永远清零，没有总上限就成了无限重试。
const TOTAL_BUDGET_FACTOR: u32 = 20;

/// 重试预算：连续失败 + 整次下载总额两条线。
///
/// - 每推进一次数据（真拿到新字节）连续计数清零，几小时的下载不会因为零星抖动
///   把额度耗光（2026-10-28 修，见 `docs/known-issues.md` §4.1）；
/// - 总额度防「每次给一点就断」的无限重试循环。
pub(super) struct RetryBudget {
    consecutive: u32,
    total: u32,
    max_consecutive: u32,
    max_total: u32,
}

impl RetryBudget {
    pub(super) fn new(max_consecutive: u32) -> Self {
        Self {
            consecutive: 0,
            total: 0,
            max_consecutive,
            max_total: max_consecutive.saturating_mul(TOTAL_BUDGET_FACTOR),
        }
    }

    /// 已经连续失败几次（提示里的 `n/max` 用它）。
    pub(super) fn consecutive(&self) -> u32 {
        self.consecutive
    }

    pub(super) fn max_consecutive(&self) -> u32 {
        self.max_consecutive
    }

    /// 额度用完没有（连续或总额任一用尽）。
    pub(super) fn exhausted(&self) -> bool {
        self.consecutive >= self.max_consecutive || self.total >= self.max_total
    }

    /// 刚拿到新字节：连续计数清零（总额不清零）。
    pub(super) fn progressed(&mut self) {
        self.consecutive = 0;
    }

    /// 申请一次重试；返回本次重试的序号（0 起，决定等待多久）。`None` = 没额度了。
    pub(super) fn grant(&mut self) -> Option<u32> {
        if self.exhausted() {
            return None;
        }
        self.consecutive += 1;
        self.total += 1;
        Some(self.consecutive - 1)
    }
}

/// 官方明确说是 0 字节：不发 GET，直接在本地落一个 0 字节文件。
///
/// 仍然走 `claim_part` 的「不覆盖 + `.kichi-part` 原子占位」，
/// 完成（`sync_all` + rename）后才算成功，和普通下载的落盘约定一致。
/// 登出命中时删掉 0 字节 part，不保留无意义的断点。
pub(super) fn save_empty_file(
    target_dir: &Path,
    name: &str,
    cancel: &Cancel,
) -> Result<Downloaded, Failure> {
    if cancel.is_cancelled() {
        return Err(Failure::Other("下载已取消".to_owned()));
    }
    std::fs::create_dir_all(target_dir)
        .map_err(|err| Failure::Other(format!("创建下载目录失败：{err}")))?;
    let (file, part_path, final_path) = claim_part(target_dir, name)?;
    if let Err(err) = file.sync_all() {
        drop(file);
        let _ = std::fs::remove_file(&part_path);
        return Err(Failure::Other(format!("写入本地文件失败：{err}")));
    }
    let commit = commit_lock();
    if cancel.is_abandoned() {
        drop(commit);
        drop(file);
        let _ = std::fs::remove_file(&part_path);
        return Err(Failure::Other("下载已取消".to_owned()));
    }
    drop(file);
    let renamed = std::fs::rename(&part_path, &final_path);
    drop(commit);
    renamed.map_err(|err| {
        let _ = std::fs::remove_file(&part_path);
        Failure::Other(format!("保存下载文件失败：{err}"))
    })?;
    Ok(Downloaded {
        path: final_path,
        bytes: 0,
    })
}

/// 准备好的临时文件：要么已经完整（只需改名），要么可以继续写。
pub(super) enum Prepared {
    Complete(Downloaded),
    Streaming(Part),
}

/// 一个正在写的 `.kichi-part`。
pub(super) struct Part {
    pub(super) file: File,
    pub(super) part_path: PathBuf,
    pub(super) final_path: PathBuf,
    /// 已写入的绝对字节数（以磁盘实际长度为准）
    pub(super) offset: u64,
    /// 完整文件大小（可能未知）
    pub(super) total: Option<u64>,
}

impl Part {
    /// 以磁盘实际长度为准（半块写失败时内存计数不可信）。
    pub(super) fn disk_len(&self) -> io::Result<u64> {
        self.file.metadata().map(|meta| meta.len())
    }

    pub(super) fn truncate(&mut self) -> io::Result<()> {
        self.file.set_len(0)?;
        self.file.seek(SeekFrom::Start(0))?;
        self.offset = 0;
        Ok(())
    }

    pub(super) fn remember(&self, file_id: &str) {
        partials::remember(&PartialEntry::new(
            file_id,
            &self.part_path,
            &self.final_path,
            self.total,
        ));
    }

    /// 取消 / 本地写坏：删临时文件并忘掉索引。
    pub(super) fn cleanup(self, file_id: &str) {
        drop(self.file);
        if let Err(err) = std::fs::remove_file(&self.part_path) {
            if err.kind() != io::ErrorKind::NotFound {
                eprintln!(
                    "[kichi] 删除临时下载文件失败（{}）：{err}",
                    self.part_path.display()
                );
            }
        }
        partials::forget(file_id);
    }

    /// 取消 / 登出导致本次中止。
    ///
    /// - 登出（abandon）：刷新索引并保留 part，下次登录续传；
    /// - 用户取消：删 part + 忘索引（ADR 0011 不变）。
    pub(super) fn abort(self, file_id: &str, cancel: &Cancel) -> Failure {
        if cancel.is_abandoned() {
            self.remember(file_id);
        } else {
            self.cleanup(file_id);
        }
        Failure::Other("下载已取消".to_owned())
    }
}

/// 选断点 / 建 part / 打开写句柄。已经完整时直接改名并返回成功。
pub(super) fn prepare_part(
    file_id: &str,
    name: &str,
    target_dir: &Path,
    remote_size: Option<u64>,
    cancel: &Cancel,
) -> Result<Prepared, Failure> {
    if cancel.is_cancelled() {
        return Err(Failure::Other("下载已取消".to_owned()));
    }
    let entry = partials::lookup(file_id, target_dir);
    let part_len = entry
        .as_ref()
        .and_then(|entry| std::fs::metadata(entry.part_path()).ok())
        .map(|meta| meta.len())
        .unwrap_or(0);
    let mut plan = plan_resume(entry.as_ref(), remote_size, part_len);
    // v2 分片的 part 是稀疏文件，长度不等于「已下载到哪」；顺序路径绝不能
    // 直接按 part_len 追加。正常流程会先在 shards.rs 处理 v2 条目，这里只是
    // 防御性兜底（例如索引损坏 / 手动把 CONNECTIONS 设成 1 的极端路径）。
    if entry.as_ref().is_some_and(|entry| !entry.shards.is_empty()) {
        plan = super::partials::ResumePlan {
            offset: 0,
            truncate: true,
            total: plan.total,
            complete: false,
        };
    }

    if let Some(entry) = entry {
        let part_path = entry.part_path();
        let final_path = entry.final_path();
        if plan.complete {
            let file = OpenOptions::new()
                .write(true)
                .open(&part_path)
                .map_err(local_write_failure)?;
            if let Err(err) = file.sync_all() {
                drop(file);
                let _ = std::fs::remove_file(&part_path);
                partials::forget(file_id);
                return Err(local_write_failure(err));
            }
            drop(file);
            // sync 后与登出作废互斥：命中就原样保留 part + 索引，下次登录直接改名
            let commit = commit_lock();
            if cancel.is_abandoned() {
                drop(commit);
                return Err(Failure::Other("下载已取消".to_owned()));
            }
            let renamed = finalize_part(&part_path, &final_path);
            drop(commit);
            let path = renamed?;
            partials::forget(file_id);
            return Ok(Prepared::Complete(Downloaded {
                path,
                bytes: plan.total.unwrap_or(part_len),
            }));
        }

        let mut file = OpenOptions::new()
            .write(true)
            .open(&part_path)
            .map_err(local_write_failure)?;
        if plan.truncate {
            if let Err(err) = file
                .set_len(0)
                .and_then(|()| file.seek(SeekFrom::Start(0)).map(|_| ()))
            {
                drop(file);
                let _ = std::fs::remove_file(&part_path);
                partials::forget(file_id);
                return Err(local_write_failure(err));
            }
        }
        let offset = file.metadata().map_err(local_write_failure)?.len();
        file.seek(SeekFrom::Start(offset))
            .map_err(local_write_failure)?;
        let part = Part {
            file,
            part_path,
            final_path,
            offset,
            total: plan.total,
        };
        // 即使是续传也刷新索引（total / 时间戳），进程中途退出后还能接着下
        part.remember(file_id);
        return Ok(Prepared::Streaming(part));
    }

    let (file, part_path, final_path) = claim_part(target_dir, name)?;
    let mut file = file;
    file.seek(SeekFrom::Start(0)).map_err(local_write_failure)?;
    let part = Part {
        file,
        part_path,
        final_path,
        offset: 0,
        total: remote_size,
    };
    part.remember(file_id);
    Ok(Prepared::Streaming(part))
}

/// 一次流式写入的结果。
pub(super) enum ReadOutcome {
    Complete,
    /// 网络类错误：可以重试，part 保留
    Retryable(String),
    /// 本地写盘错误：删 part，不再重试
    Local(String),
    Cancelled,
}

/// 读到 EOF 后按总大小判定结果（泛型只为单测能塞 `Cursor`）：
///
/// - 已知 `total`：提前结束可重试（保留 part），超出按本地错误删 part；
/// - **未知 `total`**：算完整。这不是放水：HTTP 的合法 EOF 只有「chunked
///   结束块」与「连接正常关闭」两种，chunked 被截断、带 `Content-Length`
///   却发不满的响应都会在读时报错（`dl_tests.rs` 的
///   `body_cut_off_error_text_never_contains_the_url` /
///   `truncated_chunked_body_is_an_error_not_eof`），走不到这里。剩下的
///   就是「服务器没说长度、把字节发完了」，没有可比的依据。
pub(super) fn read_into_part<S: Read>(
    stream: &mut S,
    part: &mut Part,
    progress: &Progress<'_>,
    cancel: &Cancel,
) -> ReadOutcome {
    let mut buffer = vec![0u8; CHUNK];
    loop {
        if cancel.is_cancelled() {
            return ReadOutcome::Cancelled;
        }
        let read = match stream.read(&mut buffer) {
            Ok(read) => read,
            Err(err) => {
                // 卡在读超时的那几十秒里按的取消仍然算「已取消」
                if cancel.is_cancelled() {
                    return ReadOutcome::Cancelled;
                }
                let reason = if err.kind() == io::ErrorKind::TimedOut {
                    format!("下载超时：{err}")
                } else {
                    format!("下载中断：{err}")
                };
                return ReadOutcome::Retryable(reason);
            }
        };
        if read == 0 {
            break;
        }
        if let Err(err) = part.file.write_all(&buffer[..read]) {
            return ReadOutcome::Local(format!("写入本地文件失败：{err}"));
        }
        part.offset += read as u64;
        progress(part.offset, part.total);
    }
    match part.total {
        Some(total) if part.offset < total => ReadOutcome::Retryable(format!(
            "连接提前结束：收到 {} / {}",
            readable_size(part.offset),
            readable_size(total)
        )),
        Some(total) if part.offset > total => {
            ReadOutcome::Local("下载长度超出预期，已放弃这个临时文件".to_owned())
        }
        _ => ReadOutcome::Complete,
    }
}

/// 打开下载流失败时该怎么做（纯函数，方便单测矩阵）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OpenAction {
    /// 地址 / 凭据等不可重试：失败，断点保留
    Fatal,
    /// 网络类：等一会儿再试；`refresh` = 重取详情换新链接
    Retry { refresh: bool },
    /// Range 位置不可用：截断 part 从 0 立刻重来
    RestartFromZero,
}

pub(super) fn classify_open_error(err: &CoreError, offset: u64) -> OpenAction {
    match err {
        CoreError::RangeNotSatisfiable { .. } if offset > 0 => OpenAction::RestartFromZero,
        CoreError::RangeNotSatisfiable { .. } => OpenAction::Fatal,
        CoreError::Api { code, .. } => match code {
            403 | 410 => OpenAction::Retry { refresh: true },
            429 => OpenAction::Retry { refresh: false },
            500..=599 => OpenAction::Retry { refresh: false },
            _ => OpenAction::Fatal,
        },
        CoreError::Network(_) | CoreError::DownloadTimeout(_) => {
            OpenAction::Retry { refresh: false }
        }
        _ => OpenAction::Fatal,
    }
}

/// 第 `attempt` 次重试（0 起）前等多久。
pub(super) fn retry_delay(attempt: u32) -> Duration {
    let index = usize::try_from(attempt).unwrap_or(usize::MAX);
    RETRY_DELAYS
        .get(index)
        .copied()
        .unwrap_or_else(|| *RETRY_DELAYS.last().unwrap_or(&Duration::from_secs(8)))
}

/// 自动重试次数（0 = 关闭）；非法 / 空串回默认。
pub(super) fn download_retries() -> u32 {
    std::env::var(RETRIES_ENV)
        .ok()
        .and_then(|raw| raw.trim().parse::<u32>().ok())
        .unwrap_or(DEFAULT_RETRIES)
}

pub(super) fn local_write_failure(err: io::Error) -> Failure {
    Failure::Other(format!("写入本地文件失败：{err}"))
}

/// 失败行带上断点提示；0 字节不硬凑「可重试续传」。
pub(super) fn resume_failure(reason: String, downloaded: u64) -> Failure {
    if downloaded == 0 {
        Failure::Other(reason)
    } else {
        Failure::Other(format!(
            "{reason}（已下载 {}，可重试续传）",
            readable_size(downloaded)
        ))
    }
}
