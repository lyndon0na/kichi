//! 下载线程的执行体：递归读目录、逐条下载、进度节流、结果回投。
//!
//! 调度与占用表在 [`super::queue`]；本文件只在后台跑，不碰 Qt 对象，
//! 所有界面更新都经 `actions::enqueue_ui` 回主线程。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kichi_core::{EntryKind, FileEntry};

use super::queue::{add_result, current_epoch, BatchReport, Job, JobOutcome, MAX_DEPTH};
use super::transfers;
use crate::session::download::{Cancel, DownloadCallbacks};
use crate::session::{Failure, Session};

/// 进度回投的最小间隔。
pub(crate) const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

/// 递归收集文件夹里的文件（保留层级；阻塞）。
pub(crate) fn collect_folder(
    session: &Arc<Session>,
    folder: &FileEntry,
    cancel: &Cancel,
) -> Result<Vec<Job>, Failure> {
    let mut jobs = Vec::new();
    let mut path = vec![folder.name.clone()];
    walk(session, &folder.id, &mut path, &mut jobs, 0, cancel)?;
    Ok(jobs)
}

fn walk(
    session: &Arc<Session>,
    folder_id: &str,
    path: &mut Vec<String>,
    jobs: &mut Vec<Job>,
    depth: usize,
    cancel: &Cancel,
) -> Result<(), Failure> {
    if depth > MAX_DEPTH {
        return Err(Failure::Other(format!(
            "目录层级超过 {MAX_DEPTH} 层，为避免异常结构已停止读取"
        )));
    }
    // 每页之间查取消：读完整个目录可能要好几秒
    for entry in session.list_dir_with_cancel(Some(folder_id), Some(cancel))? {
        if cancel.is_cancelled() {
            return Err(Failure::Other("下载已取消".to_owned()));
        }
        match entry.kind {
            EntryKind::Folder => {
                path.push(entry.name.clone());
                walk(session, &entry.id, path, jobs, depth + 1, cancel)?;
                path.pop();
            }
            EntryKind::File => jobs.push(Job {
                id: entry.id.clone(),
                sub_dir: path.clone(),
                name: entry.name.clone(),
                size: entry.size,
                audit_flagged: entry.audit_flagged,
            }),
        }
    }
    Ok(())
}

/// 下载线程：逐条下载，结果回投主线程。
///
/// 整个循环包在 `catch_unwind` 里：panic 也必须走到收尾（标失败 / 释放占用 /
/// 推进下一批），不能让一批永远占着 `current`。收尾本身在 [`post_finished`]
/// 里，只走一次，所以一批最多一条系统通知。
pub(crate) fn run_jobs(
    session: Arc<Session>,
    dir: PathBuf,
    batch: u64,
    jobs: Vec<Job>,
    cancel: Arc<Cancel>,
    my_epoch: u64,
) {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        download_all(&session, &dir, batch, &jobs, &cancel, my_epoch)
    }));
    let interrupted = outcome.is_err();
    if interrupted {
        // 记录表里这一批还活着的行都标失败；通知统一由 post_finished 发
        transfers::fail_batch(batch, "下载线程异常中断");
    }
    // 取消 / 登出导致提前退出时，还没轮到的条目也要收尾。
    // 取消时当前条可能刚好下载成功，也会落在这里。
    if cancel.is_cancelled() {
        transfers::cancel_batch(batch, "");
    }
    // 优先级：panic > 取消 > 正常完成。fail_batch 已经写下的失败原因
    // 不会被 cancel_batch 覆盖（后者只动活动记录，失败行已经结束了）。
    let end = if interrupted {
        BatchEnd::Interrupted
    } else if cancel.is_cancelled() {
        BatchEnd::Cancelled
    } else {
        BatchEnd::Completed
    };
    post_finished(batch, end, my_epoch);
}

/// 逐条下载（[`run_jobs`] 的主体；panic 由调用方兜底）。
fn download_all(
    session: &Arc<Session>,
    dir: &Path,
    batch: u64,
    jobs: &[Job],
    cancel: &Arc<Cancel>,
    my_epoch: u64,
) {
    for (index, job) in jobs.iter().enumerate() {
        if my_epoch != current_epoch() || cancel.is_cancelled() {
            break;
        }
        transfers::begin(batch, index);
        post_sync();

        let mut target = dir.to_path_buf();
        for part in &job.sub_dir {
            target.push(crate::session::download::safe_file_name(part));
        }
        let throttle = Throttle::default();
        let progress = |bytes: u64, total: Option<u64>| {
            if throttle.should_send(bytes, total) {
                transfers::progress(batch, index, bytes, total);
                post_live();
            }
        };
        let note = |text: &str| {
            transfers::note(batch, index, text);
            // 说明不是结构变化：走 live 贴到当前文件行（第二行）上。
            // 以前走 post_sync 时它被下载中的第二行隐藏，等于看不见。
            post_live();
        };
        let callbacks = DownloadCallbacks {
            progress: &progress,
            note: &note,
        };
        let outcome = session.download_file(
            &job.id,
            &job.name,
            job.audit_flagged,
            &target,
            callbacks,
            cancel,
        );
        // 下载期间退出过登录：记录表已整表清空，这条结果属于上个会话，丢弃
        if my_epoch != current_epoch() {
            return;
        }
        let cancelled = cancel.is_cancelled();
        match outcome {
            Ok(downloaded) => {
                add_result(batch, JobOutcome::Saved);
                transfers::finish_ok(batch, index, &downloaded.path);
                post_sync();
            }
            Err(_) if cancelled => {
                // 取消不算失败：数量对得上总账，记录由统一收尾标成已取消
                break;
            }
            Err(err) => {
                let reason = err.to_string();
                add_result(batch, JobOutcome::Failed { reason: &reason });
                transfers::finish_err(batch, index, &reason);
                post_sync();
            }
        }
    }
}

/// 进度节流：250ms 或进 1% 才发。
///
/// 只被下载线程使用；用原子量而非 `Cell`，是为了满足进度回调的 `Send` 要求。
#[derive(Default)]
struct Throttle {
    last_sent: Mutex<Option<Instant>>,
    last_percent: AtomicI32,
}

impl Throttle {
    fn should_send(&self, bytes: u64, total: Option<u64>) -> bool {
        let percent = match total {
            Some(total) if total > 0 => (bytes as f64 / total as f64 * 100.0) as i32,
            _ => -1,
        };
        let now = Instant::now();
        let mut last = self.last_sent.lock().unwrap_or_else(|e| e.into_inner());
        let first = last.is_none();
        let due = match *last {
            Some(previous) => now.duration_since(previous) >= PROGRESS_INTERVAL,
            None => false,
        };
        let previous_percent = self.last_percent.load(Ordering::Relaxed);
        if first || due || percent != previous_percent {
            *last = Some(now);
            self.last_percent.store(percent, Ordering::Relaxed);
            true
        } else {
            false
        }
    }
}

// —— 后台线程 → 主线程（不碰 Qt 对象，只入队）——

/// 记录变了（开始 / 完成 / 失败 / 取消）→ 主线程重建模型。
fn post_sync() {
    super::actions::enqueue_ui(
        move |mut qichi: std::pin::Pin<&mut super::qobject::Kichi>| {
            qichi.as_mut().sync_transfers();
        },
    );
}

/// 只是进度变了 → 主线程只刷新进度行，不动整个 ListView 模型。
fn post_live() {
    super::actions::enqueue_ui(
        move |mut qichi: std::pin::Pin<&mut super::qobject::Kichi>| {
            qichi.as_mut().sync_transfer_live();
        },
    );
}

/// 一批结束的三态（决定通知文案与状态栏）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BatchEnd {
    Completed,
    Cancelled,
    Interrupted,
}

/// 整批结束：释放占用、最多发一条通知、给状态栏收尾，然后推进下一批。
///
/// 通知开关在这里（主线程）读，用户中途改也能生效。`my_epoch` 与当前代数
/// 不一致时说明期间退出过登录：这里什么都别碰，否则可能把重新登录后的
/// 新批次状态清掉。
fn post_finished(batch: u64, end: BatchEnd, my_epoch: u64) {
    super::actions::enqueue_ui(
        move |mut qichi: std::pin::Pin<&mut super::qobject::Kichi>| {
            if my_epoch != current_epoch() {
                return;
            }
            let Some(report) = super::queue::release(batch) else {
                return;
            };
            if qichi.notify_enabled {
                if let Some(notice) = end_notice(&report, end) {
                    super::notifications::show_notice(&notice);
                }
            }
            match end {
                BatchEnd::Cancelled => qichi.as_mut().set_message(
                    &format!(
                        "下载已取消（已完成 {} 个，失败 {} 个）",
                        report.done, report.failed
                    ),
                    false,
                ),
                BatchEnd::Completed if report.failed == 0 && report.done > 0 => qichi
                    .as_mut()
                    .set_message(&format!("下载完成：{} 个文件", report.done), false),
                // 失败 / 中断按用户选择不留在状态栏，清掉「开始下载…」之类的旧文案
                _ => qichi.as_mut().set_message("", false),
            }
            qichi.as_mut().advance_downloads();
        },
    );
}

/// 批次结束 → 一条通知；取消不弹。
fn end_notice(report: &BatchReport, end: BatchEnd) -> Option<super::notifications::Notice> {
    match end {
        BatchEnd::Cancelled => None,
        BatchEnd::Interrupted => Some(super::notifications::batch_failed(
            &report.name,
            report.folder,
            "下载线程异常中断",
        )),
        BatchEnd::Completed => Some(super::notifications::batch_finished(
            super::notifications::BatchSummary {
                name: &report.name,
                folder: report.folder,
                done: report.done,
                failed: report.failed,
                failure: report.failure.as_deref(),
            },
        )),
    }
}

#[cfg(test)]
#[path = "queue_worker_tests.rs"]
mod tests;

/// 起线程需要的一份快照（不把锁里的 `Pending` 借出去）。
pub(crate) struct Start {
    pub(crate) batch: u64,
    pub(crate) folder: bool,
    pub(crate) entry: FileEntry,
    pub(crate) dir: PathBuf,
    pub(crate) cancel: Arc<Cancel>,
    /// 单文件重试时保留的层级（`Job.sub_dir`）
    pub(crate) sub_dir: Vec<String>,
}

/// 起这一批的线程：文件夹先读目录，单文件直接进下载循环。
///
/// 两个线程体都包了 `catch_unwind`：万一中途 panic，也必须回主线程收尾，
/// 否则 `current` 永远占着，整个队列（以及那个条目 id）就再也动不了了。
pub(crate) fn spawn_batch(session: &Arc<Session>, start: &Start) -> std::io::Result<()> {
    let session = session.clone();
    let batch = start.batch;
    let cancel = start.cancel.clone();
    let my_epoch = current_epoch();
    let builder = std::thread::Builder::new().name("kichi-dl".into());
    if start.folder {
        let folder = start.entry.clone();
        builder
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    collect_folder(&session, &folder, &cancel)
                }))
                .unwrap_or_else(|_| Err(Failure::Other("读取目录时后台任务异常中断".to_owned())));
                let cancelled = cancel.is_cancelled();
                // 读完目录回主线程：展开记录、进下载循环，或者标掉这一批
                super::actions::enqueue_ui(move |mut qichi| {
                    qichi
                        .as_mut()
                        .on_folder_collected(batch, result, cancelled, my_epoch);
                });
            })
            .map(|_| ())
    } else {
        let job = Job {
            id: start.entry.id.clone(),
            sub_dir: start.sub_dir.clone(),
            name: start.entry.name.clone(),
            size: start.entry.size,
            audit_flagged: start.entry.audit_flagged,
        };
        let dir = start.dir.clone();
        builder
            .spawn(move || run_jobs(session, dir, batch, vec![job], cancel, my_epoch))
            .map(|_| ())
    }
}
