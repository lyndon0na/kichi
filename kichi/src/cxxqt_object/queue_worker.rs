//! 下载线程的执行体：建目录、逐条下载、进度节流、结果回投。
//!
//! 目录遍历在 [`super::queue_collect`]（空文件夹 / 过深 / 收集期失败的边界
//! 见 `docs/adr/0012-folder-download-boundaries.md`）；调度与占用表在
//! [`super::queue`]；本文件只在后台跑，不碰 Qt 对象，
//! 所有界面更新都经 `actions::enqueue_ui` 回主线程。
//!
//! 登出后旧线程只做收尾：`post_finished` 即使 epoch 已变也必须还回调度槽，
//! 让新会话排队的批次在旧线程退出后开始。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kichi_core::FileEntry;

use super::queue::{add_result, current_epoch, BatchReport, Job, JobOutcome};
use super::queue_collect::{self, Collected};
use super::transfers;
use crate::session::download::{safe_file_name, Cancel, DownloadCallbacks};
use crate::session::{Failure, Session};

/// 进度回投的最小间隔。
pub(crate) const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

/// 相对下载根目录的目录路径 → 本地绝对路径。
///
/// 建目录与文件落盘必须共用这一套 `safe_file_name` 拼接（`docs/pitfalls.md`），
/// 否则目录建在一个名字下、文件落到另一个名字下。
pub(crate) fn local_dir(root: &Path, parts: &[String]) -> PathBuf {
    let mut path = root.to_path_buf();
    for part in parts {
        path.push(safe_file_name(part));
    }
    path
}

/// 先把收集到的目录树建出来；返回（新增跳过数, 根目录失败的致命原因）。
///
/// 根目录建不出来 = 整批失败（后面所有文件都落不下去）；非根失败只并入跳过计数，
/// 文件下载本身还会 `create_dir_all`，不依赖这里预建成功。
fn create_dirs(root: &Path, dirs: &[Vec<String>], cancel: &Cancel) -> (usize, Option<String>) {
    let mut skipped = 0;
    let mut fatal = None;
    for (index, parts) in dirs.iter().enumerate() {
        if cancel.is_cancelled() {
            break;
        }
        let path = local_dir(root, parts);
        if let Err(err) = std::fs::create_dir_all(&path) {
            if index == 0 {
                fatal = Some(format!("创建下载目录失败：{err}"));
                break;
            }
            skipped += 1;
        }
    }
    (skipped, fatal)
}

/// 下载线程：建目录、逐条下载，结果回投主线程。
///
/// 整个循环包在 `catch_unwind` 里：panic 也必须走到收尾（标失败 / 释放占用 /
/// 推进下一批），不能让一批永远占着 `current`。收尾本身在 [`post_finished`]
/// 里，只走一次，所以一批最多一条系统通知。
pub(crate) fn run_jobs(
    session: Arc<Session>,
    dir: PathBuf,
    batch: u64,
    collected: Collected,
    cancel: Arc<Cancel>,
    my_epoch: u64,
) {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        download_all(&session, &dir, batch, &collected, &cancel, my_epoch)
    }));
    let interrupted = outcome.is_err();
    if interrupted {
        // 记录表里这一批还活着的行都标失败；通知统一由 post_finished 发
        transfers::fail_batch(batch, "下载线程异常中断");
    }
    // `download_all` 返回的批次级失败原因（建目录失败）：标掉还没完成的记录。
    // 取消优先：期间按过取消就按取消收尾，别把行写成失败。
    let batch_error = match &outcome {
        Ok(reason) if !cancel.is_cancelled() => reason.clone(),
        _ => None,
    };
    if let Some(reason) = &batch_error {
        transfers::fail_batch(batch, reason);
    }
    // 取消 / 登出导致提前退出时，还没轮到的条目也要收尾。
    // 取消时当前条可能刚好下载成功，也会落在这里。
    if cancel.is_cancelled() {
        transfers::cancel_batch(batch, "");
    }
    // 优先级：panic > 取消 > 批次级失败 > 正常完成。fail_batch 已经写下的失败原因
    // 不会被 cancel_batch 覆盖（后者只动活动记录，失败行已经结束了）。
    let end = if interrupted {
        BatchEnd::Interrupted
    } else if cancel.is_cancelled() {
        BatchEnd::Cancelled
    } else if let Some(reason) = batch_error {
        BatchEnd::Failed(reason)
    } else {
        BatchEnd::Completed
    };
    post_finished(batch, end, my_epoch);
}

/// 建目录 + 逐条下载（[`run_jobs`] 的主体；panic 由调用方兜底）。
///
/// 返回批次级失败原因（只有根目录建不出来）：这时这一批算整体失败，
/// 而不是「完成了但跳过很多」。
fn download_all(
    session: &Arc<Session>,
    dir: &Path,
    batch: u64,
    collected: &Collected,
    cancel: &Arc<Cancel>,
    my_epoch: u64,
) -> Option<String> {
    let (dir_skipped, fatal) = create_dirs(dir, &collected.dirs, cancel);
    if dir_skipped > 0 {
        super::queue::add_skipped(batch, dir_skipped);
        transfers::add_skipped(batch, dir_skipped);
    }
    if fatal.is_some() {
        return fatal;
    }
    if cancel.is_cancelled() {
        return None;
    }
    if collected.files.is_empty() {
        // 空文件夹（可能带被跳过的分支）：目录已建好，这一批算完成
        transfers::finish_folder_empty(batch);
        post_sync();
        return None;
    }

    for (index, job) in collected.files.iter().enumerate() {
        if my_epoch != current_epoch() || cancel.is_cancelled() {
            break;
        }
        transfers::begin(batch, index);
        post_sync();

        let target = local_dir(dir, &job.sub_dir);
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
            return None;
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
    None
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

/// 一批结束的四种去向（决定通知文案与状态栏）。
#[derive(Debug, Clone, PartialEq, Eq)]
enum BatchEnd {
    Completed,
    Cancelled,
    Interrupted,
    /// 批次级失败（根目录建不出来）：原因写进记录与通知
    Failed(String),
}

/// 整批结束：释放占用、最多发一条通知、给状态栏收尾，然后推进下一批。
///
/// 通知开关在这里（主线程）读，用户中途改也能生效。`my_epoch` 与当前代数
/// 不一致时说明期间退出过登录：不碰记录 / 通知 / 状态栏（那些已属于上个会话），
/// 但**必须还回调度槽**——旧线程真正退出后，新会话排队的批次才能开始。
fn post_finished(batch: u64, end: BatchEnd, my_epoch: u64) {
    super::actions::enqueue_ui(
        move |mut qichi: std::pin::Pin<&mut super::qobject::Kichi>| {
            if my_epoch != current_epoch() {
                // 登出后旧线程收尾：把调度槽还回来，新会话排队的批次才能开始
                let _ = super::queue::release(batch);
                qichi.as_mut().advance_downloads();
                return;
            }
            let Some(report) = super::queue::release(batch) else {
                return;
            };
            if qichi.notify_enabled {
                if let Some(notice) = end_notice(&report, &end) {
                    super::notifications::show_notice(&notice);
                }
            }
            // 状态栏文案由纯函数决定：空文件夹 = 完成，失败 / 中断不进状态栏
            match status_text(&report, &end) {
                Some(text) => qichi.as_mut().set_message(&text, false),
                None => qichi.as_mut().set_message("", false),
            }
            qichi.as_mut().advance_downloads();
        },
    );
}

/// 批次结束 → 状态栏文案（`None` = 清空，失败 / 中断不留旧文案）。
fn status_text(report: &BatchReport, end: &BatchEnd) -> Option<String> {
    match end {
        BatchEnd::Cancelled => Some(format!(
            "下载已取消（已完成 {} 个，失败 {} 个）",
            report.done, report.failed
        )),
        BatchEnd::Completed if report.empty_folder => {
            Some(format!("已创建空文件夹：{}", report.name))
        }
        BatchEnd::Completed if report.done > 0 && report.failed == 0 => {
            let mut text = format!("下载完成：{} 个文件", report.done);
            if report.skipped > 0 {
                text.push_str(&format!("（跳过 {} 个目录）", report.skipped));
            }
            Some(text)
        }
        // 失败 / 中断 / 部分失败按用户选择不留在状态栏
        _ => None,
    }
}

/// 批次结束 → 一条通知；取消与空文件夹不弹。
fn end_notice(report: &BatchReport, end: &BatchEnd) -> Option<super::notifications::Notice> {
    match end {
        BatchEnd::Cancelled => None,
        BatchEnd::Interrupted => Some(super::notifications::batch_failed(
            &report.name,
            report.folder,
            "下载线程异常中断",
        )),
        BatchEnd::Failed(reason) => Some(super::notifications::batch_failed(
            &report.name,
            report.folder,
            reason,
        )),
        // 空文件夹是成功，但不值得打扰：验收点见 ADR 0012
        BatchEnd::Completed if report.empty_folder => None,
        BatchEnd::Completed => Some(super::notifications::batch_finished(
            super::notifications::BatchSummary {
                name: &report.name,
                folder: report.folder,
                done: report.done,
                failed: report.failed,
                skipped: report.skipped,
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
        let spawned = builder.spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                queue_collect::collect_folder(&session, &folder, &cancel)
            }))
            .unwrap_or_else(|_| Err(Failure::Other("读取目录时后台任务异常中断".to_owned())));
            let cancelled = cancel.is_cancelled();
            // 读完目录回主线程：展开记录、进下载循环，或者标掉这一批
            super::actions::enqueue_ui(move |mut qichi| {
                qichi
                    .as_mut()
                    .on_folder_collected(batch, result, cancelled, my_epoch);
            });
        });
        if spawned.is_ok() {
            super::queue::mark_started(batch);
        }
        spawned.map(|_| ())
    } else {
        let job = Job {
            id: start.entry.id.clone(),
            sub_dir: start.sub_dir.clone(),
            name: start.entry.name.clone(),
            size: start.entry.size,
            audit_flagged: start.entry.audit_flagged,
        };
        let dir = start.dir.clone();
        // 单文件批次没有目录树要建：sub_dir 已由下载循环自己 create_dir_all
        let collected = Collected {
            files: vec![job],
            dirs: Vec::new(),
            skipped: 0,
        };
        let spawned =
            builder.spawn(move || run_jobs(session, dir, batch, collected, cancel, my_epoch));
        if spawned.is_ok() {
            super::queue::mark_started(batch);
        }
        spawned.map(|_| ())
    }
}
