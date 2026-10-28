//! 下载到本地：取链接 → 流式写盘。
//!
//! 职责边界（守「会话状态只放 session」的纪律）：
//! - 本模块只做「一个文件的一次下载」，不排队、不弹通知、不碰界面；
//! - 队列、进度汇总、通知属于界面侧调度，放 `cxxqt_object::queue`；
//! - 全部函数阻塞，调用方负责丢到后台线程。
//!
//! 落盘约定见 [`download_naming`]：先写 `<名>.kichi-part`，**下载完整成功后才
//! rename** 成正式文件名。中途取消删除临时文件；网络失败 / 登出 / 退出保留
//! part + 索引，下次续传；**登出后绝不再 rename 出成品**。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use kichi_core::dl::download_idle;

// 文件名策略在 `download_naming`（那里有单测），这里转出去给队列层用。
use super::download_naming::finalize_part;
pub use super::download_naming::safe_file_name;
use super::download_resume::{
    classify_open_error, download_retries, local_write_failure, prepare_part, read_into_part,
    resume_failure, retry_delay, save_empty_file, OpenAction, Part, Prepared, ReadOutcome,
};
use super::media::{file_info, file_info_for_download, KnownHint};
use super::partials::{self, PartialEntry};
use super::{Failure, Session};
use crate::format::size as readable_size;

/// 可取消等待的检查间隔（最坏多等这么久就发现取消）。
const CANCEL_POLL: Duration = Duration::from_millis(50);
/// 一次下载的结果。
#[derive(Debug, Clone)]
pub struct Downloaded {
    /// 最终落盘路径（已改名，不含临时后缀）
    pub path: PathBuf,
    pub bytes: u64,
}

/// 取消信号：下载线程每块检查一次。
#[derive(Debug, Default)]
pub struct Cancel {
    flag: AtomicBool,
    /// 手动登出：停止 + 保留 part（与用户取消的「删 part」相反）
    abandon: AtomicBool,
}

impl Cancel {
    /// 是否已请求取消。
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::Acquire)
    }

    /// 是否因登出而中止（命中时保留断点，绝不 rename 成品）。
    pub fn is_abandoned(&self) -> bool {
        self.abandon.load(Ordering::Relaxed)
    }

    /// 请求取消（任意线程可调）。只停止，不改变「是否保留 part」。
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::Relaxed);
    }

    /// 登出：停止并保留断点。
    ///
    /// 先写 `abandon` 再写停止标志（Release）：任何看到「已停止」的线程
    /// 都能看到登出标记，提交点的最后检查不会漏。
    pub fn abandon(&self) {
        self.abandon.store(true, Ordering::Relaxed);
        self.flag.store(true, Ordering::Release);
    }
}

/// 「最后检查登出 + rename」与「登出置 abandon」互斥。
///
/// 保证：要么文件在登出开始前已经完整 rename；要么登出开始后一定不 rename。
/// 锁序固定为 `commit_lock → scheduler`，绝不反向。
pub(crate) fn commit_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 可被取消打断的等待：等到 `total` 或期间被取消。`false` = 被取消。
///
/// 按 50 ms 切片查标志，取消最坏多等一个切片；不用条件变量是因为
/// `Cancel` 只是个原子标志，调用方（界面线程）不该为此持有锁。
pub fn sleep_cancellable(cancel: &Cancel, total: Duration) -> bool {
    let started = Instant::now();
    loop {
        if cancel.is_cancelled() {
            return false;
        }
        let remaining = total.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return true;
        }
        std::thread::sleep(remaining.min(CANCEL_POLL));
    }
}

impl Session {
    /// 下载一个文件到 `target_dir`（目录不存在会自动创建）。阻塞。
    ///
    /// `name_hint` 是列表里的文件名；官方详情里也可能给名字，优先用列表的
    /// （列表是用户眼前看到的那个）。`progress` 每读一块调一次。
    ///
    /// `hint` 来自文件列表 / 下载队列：详情没下发 `size` 时用它补总大小，
    /// `audit_flagged` 则用来判断「违规 / 受限」并跳过没用的 5 秒重试。
    /// 详情明确 `size == 0` 时 hint 不生效。
    ///
    /// 详情明确 `size == 0` 时不取链接，直接落一个 0 字节文件并算成功
    /// （对齐 rclone 的 `Open`）。
    ///
    /// 断点续传（`docs/adr/0011-resumable-downloads.md`）：
    /// - 失败 / 登出 / 退出时保留 `.kichi-part` 与索引，下次同一个云端 id 从断点继续；
    /// - 登出后即使数据已经读完也不 rename 成品；
    /// - 用户取消 / 本地写盘失败删除临时文件；
    /// - 网络类错误自动重试（默认 3 次，1s / 3s / 8s），403/410 重取一次详情换链接。
    pub fn download_file(
        &self,
        file_id: &str,
        name_hint: &str,
        hint: KnownHint,
        target_dir: &Path,
        callbacks: DownloadCallbacks<'_>,
        cancel: &Cancel,
    ) -> Result<Downloaded, Failure> {
        let progress = callbacks.progress;
        let note = callbacks.note;
        // 官方没有「只取链接」的轻量接口：这次详情请求省不掉（直链只在详情里）。
        // 列表已知大小只用来补总大小，0 字节 / 违规判定仍看详情。
        let info = file_info_for_download(self, file_id, hint, Some(cancel))?;
        let name = safe_file_name(if name_hint.trim().is_empty() {
            &info.name
        } else {
            name_hint
        });
        // 取消后不再建连接：这一步可能白等响应头
        if cancel.is_cancelled() {
            return Err(Failure::Other("下载已取消".to_owned()));
        }
        // 0 字节：官方不会给内容，也不必为此发一次 GET。
        // 不调用 progress(0, Some(0))：传输页进行中行会显示「0 B / 0 B」，
        // 不调用时完成行直接显示「0 B」。
        if info.is_zero_byte() {
            return save_empty_file(target_dir, &name, cancel);
        }
        // 没有可用的下载链接：先报错，别留下一个空的 .kichi-part 和索引
        if info.download_url().is_none() {
            return Err(Failure::Other(
                "官方没有下发这个文件的下载链接（可能是违规或受限文件）".to_owned(),
            ));
        }
        std::fs::create_dir_all(target_dir)
            .map_err(|err| Failure::Other(format!("创建下载目录失败：{err}")))?;
        // 大文件先试多分片；不分片 / 服务器不支持时返回 None，走下面的单连接。
        if let Some(result) = super::shards::download(
            self, &info, hint, file_id, &name, target_dir, callbacks, cancel,
        ) {
            return result;
        }
        let prepared = prepare_part(file_id, &name, target_dir, info.size, cancel)?;
        let mut part = match prepared {
            Prepared::Complete(downloaded) => return Ok(downloaded),
            Prepared::Streaming(part) => part,
        };

        let max_retries = download_retries();
        let mut retries: u32 = 0;
        let mut info = info;
        let mut first_attempt = true;

        loop {
            if cancel.is_cancelled() {
                return Err(part.abort(file_id, cancel));
            }
            let offset = match part.disk_len() {
                Ok(offset) => offset,
                Err(err) => {
                    part.cleanup(file_id);
                    return Err(local_write_failure(err));
                }
            };
            part.offset = offset;
            if first_attempt {
                first_attempt = false;
                if offset > 0 {
                    note(&format!("已从 {} 处继续", readable_size(offset)));
                }
            }
            progress(offset, part.total);

            // 每次按令牌命中会话缓存，不新建连接池（真正的建池在 `drive()` 里）。
            let opened = self
                .drive()?
                .open_download_from_with(&info, offset, download_idle());
            let mut stream = match opened {
                Ok(stream) => stream,
                Err(err) => {
                    // 卡在读超时的那几十秒里按的取消仍然算「已取消」
                    if cancel.is_cancelled() {
                        return Err(part.abort(file_id, cancel));
                    }
                    match classify_open_error(&err, offset) {
                        OpenAction::RestartFromZero => {
                            if let Err(err) = part.truncate() {
                                part.cleanup(file_id);
                                return Err(local_write_failure(err));
                            }
                            note("服务器要求从头下载");
                            continue;
                        }
                        OpenAction::Retry { refresh } => {
                            if retries >= max_retries {
                                part.remember(file_id);
                                return Err(resume_failure(err.to_string(), offset));
                            }
                            retries += 1;
                            let delay = retry_delay(retries - 1);
                            let head = if refresh {
                                "下载链接已过期"
                            } else {
                                "连接中断"
                            };
                            note(&format!(
                                "{head}，{} 秒后从 {} 继续（{}/{}）",
                                delay.as_secs(),
                                readable_size(offset),
                                retries,
                                max_retries
                            ));
                            if !sleep_cancellable(cancel, delay) {
                                return Err(part.abort(file_id, cancel));
                            }
                            if refresh {
                                match file_info(self, file_id, Some(cancel), hint.audit_flagged) {
                                    Ok(refreshed) => {
                                        info = refreshed;
                                        // 换链接期间云端文件可能被替换：大小变了就丢弃旧 part
                                        if let (Some(old), Some(new)) = (part.total, info.size) {
                                            if old != new {
                                                if let Err(err) = part.truncate() {
                                                    part.cleanup(file_id);
                                                    return Err(local_write_failure(err));
                                                }
                                                note("云端文件已变化，从头下载");
                                            }
                                        }
                                        part.total = info.size.or(part.total);
                                    }
                                    Err(err) => {
                                        if cancel.is_cancelled() {
                                            return Err(part.abort(file_id, cancel));
                                        }
                                        if matches!(
                                            err,
                                            Failure::Auth(_)
                                                | Failure::RateLimited { .. }
                                                | Failure::Captcha { .. }
                                        ) {
                                            part.remember(file_id);
                                            return Err(err);
                                        }
                                        if retries >= max_retries {
                                            part.remember(file_id);
                                            return Err(resume_failure(err.to_string(), offset));
                                        }
                                    }
                                }
                            }
                            continue;
                        }
                        OpenAction::Fatal => {
                            // 登录失效 / 配额 / 其它不可重试：断点保留，让用户稍后手动重试
                            part.remember(file_id);
                            return Err(Failure::from(err));
                        }
                    }
                }
            };

            // 200 表示服务器忽略了 Range：part 必须截断重写；206 的位置由 core 校验。
            let start = stream.start_offset();
            if start == 0 && offset > 0 {
                if let Err(err) = part.truncate() {
                    part.cleanup(file_id);
                    return Err(local_write_failure(err));
                }
                note("服务器不支持续传，从头下载");
            } else if start != offset {
                // core 正常不会走到这里；真遇到就按「从头」处理，别把字节写错位置
                if let Err(err) = part.truncate() {
                    part.cleanup(file_id);
                    return Err(local_write_failure(err));
                }
                note("服务器返回的续传位置不对，从头下载");
                continue;
            }
            if part.total.is_none() {
                part.total = stream.total_bytes().or(info.size);
            } else if start == 0 {
                // 服务器从头给整个文件：以这次响应的完整大小为准
                part.total = stream.total_bytes().or(info.size).or(part.total);
            }

            match read_into_part(&mut stream, &mut part, progress, cancel) {
                ReadOutcome::Complete => {
                    // 数据已齐：登出也只能保留 part，绝不 rename
                    if cancel.is_abandoned() {
                        return Err(part.abort(file_id, cancel));
                    }
                    // flush + sync 都成功才算落盘，再改名
                    if let Err(err) = part.file.flush().and_then(|()| part.file.sync_all()) {
                        part.cleanup(file_id);
                        return Err(local_write_failure(err));
                    }
                    // sync 可能耗时几秒；与登出作废互斥后再决定是否 rename
                    let commit = commit_lock();
                    if cancel.is_abandoned() {
                        drop(commit);
                        return Err(part.abort(file_id, cancel));
                    }
                    let total = part.total;
                    let Part {
                        file,
                        part_path,
                        final_path,
                        offset,
                        ..
                    } = part;
                    drop(file);
                    let result = finalize_part(&part_path, &final_path);
                    drop(commit);
                    match result {
                        Ok(path) => {
                            partials::forget(file_id);
                            return Ok(Downloaded {
                                path,
                                bytes: offset,
                            });
                        }
                        Err(err) => {
                            // 改名失败不丢已下完的数据；索引留着，重试时会走「直接改名」
                            partials::remember(&PartialEntry::new(
                                file_id,
                                &part_path,
                                &final_path,
                                total,
                            ));
                            return Err(err);
                        }
                    }
                }
                ReadOutcome::Cancelled => {
                    return Err(part.abort(file_id, cancel));
                }
                ReadOutcome::Local(reason) => {
                    // 本地写盘错误（磁盘满等）：可能有半块写坏，删 part 并忘掉索引
                    part.cleanup(file_id);
                    return Err(Failure::Other(reason));
                }
                ReadOutcome::Retryable(reason) => {
                    if retries >= max_retries {
                        part.remember(file_id);
                        return Err(resume_failure(reason, part.offset));
                    }
                    retries += 1;
                    let delay = retry_delay(retries - 1);
                    note(&format!(
                        "连接中断，{} 秒后从 {} 继续（{}/{}）",
                        delay.as_secs(),
                        readable_size(part.offset),
                        retries,
                        max_retries
                    ));
                    if !sleep_cancellable(cancel, delay) {
                        return Err(part.abort(file_id, cancel));
                    }
                }
            }
        }
    }
}

/// 下载进度回调（下载线程 / 分片线程都会调用，实现必须线程安全）。
pub type Progress<'a> = dyn Fn(u64, Option<u64>) + Send + Sync + 'a;

/// 状态 / 重试提示回调（传输行 detail；与进度同风格）。
pub type Note<'a> = dyn Fn(&str) + Send + Sync + 'a;

/// 下载过程中的两条回投路径（下载线程调用）。
///
/// 合成一个结构只是为了守住参数个数：进度与提示的语义不同，但生命周期一致。
#[derive(Clone, Copy)]
pub struct DownloadCallbacks<'a> {
    pub progress: &'a Progress<'a>,
    pub note: &'a Note<'a>,
}

#[cfg(test)]
#[path = "download_tests.rs"]
mod tests;
