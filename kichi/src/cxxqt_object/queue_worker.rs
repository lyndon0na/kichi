//! 下载线程的执行体：递归读目录、逐条下载、进度节流、结果回投。
//!
//! 调度与占用表在 [`super::queue`]；本文件只在后台跑，不碰 Qt 对象，
//! 所有界面更新都经 `actions::enqueue_ui` 回主线程。

use std::path::PathBuf;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kichi_core::{EntryKind, FileEntry};

use super::queue::{add_result, current_epoch, Job, MAX_DEPTH};
use super::transfers;
use crate::session::download::Cancel;
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
/// 推进下一批），不能让一批永远占着 `current`。回收站线程同理——
/// 收尾本身在 [`finish_batch`] 里，只走一次。
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
        // 记录表里这一批还活着的行都标失败，并告诉用户原因
        transfers::fail_batch(batch, "下载线程异常中断");
        post_failed("这一批下载", "下载线程异常中断", my_epoch);
    }
    // 取消 / 登出导致提前退出时，还没轮到的条目也要收尾。
    // 取消时当前条可能刚好下载成功，也会落在这里。
    if cancel.is_cancelled() {
        transfers::cancel_batch(batch, "");
    }
    post_finished(batch, cancel.is_cancelled(), my_epoch);
}

/// 逐条下载（[`run_jobs`] 的主体；panic 由调用方兜底）。
fn download_all(
    session: &Arc<Session>,
    dir: &PathBuf,
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

        let mut target = dir.clone();
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
        let outcome = session.download_file(
            &job.id,
            &job.name,
            job.audit_flagged,
            &target,
            &progress,
            cancel,
        );
        // 下载期间退出过登录：记录表已整表清空，这条结果属于上个会话，丢弃
        if my_epoch != current_epoch() {
            return;
        }
        let cancelled = cancel.is_cancelled();
        match outcome {
            Ok(downloaded) => {
                add_result(batch, true);
                transfers::finish_ok(batch, index, &downloaded.path);
                post_saved(&job.name, &downloaded.path, my_epoch);
            }
            Err(_) if cancelled => {
                // 取消不算失败：数量对得上总账，记录由统一收尾标成已取消
                break;
            }
            Err(err) => {
                add_result(batch, false);
                transfers::finish_err(batch, index, &err.to_string());
                post_failed(&job.name, &err.to_string(), my_epoch);
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

fn post_saved(name: &str, path: &std::path::Path, my_epoch: u64) {
    let name = name.to_owned();
    let path = path.display().to_string();
    super::actions::enqueue_ui(
        move |mut qichi: std::pin::Pin<&mut super::qobject::Kichi>| {
            // 期间退出过登录：记录已清空，不再补通知，也不把旧结果贴回去
            if my_epoch != current_epoch() {
                return;
            }
            if qichi.notify_enabled {
                super::notifications::show(
                    "下载完成",
                    &format!("{name}\n保存到 {path}"),
                    "folder-download",
                );
            }
            qichi.as_mut().sync_transfers();
        },
    );
}

fn post_failed(name: &str, reason: &str, my_epoch: u64) {
    let name = name.to_owned();
    let reason = reason.to_owned();
    super::actions::enqueue_ui(
        move |mut qichi: std::pin::Pin<&mut super::qobject::Kichi>| {
            if my_epoch != current_epoch() {
                return;
            }
            if qichi.notify_enabled {
                super::notifications::show(
                    "下载失败",
                    &format!("{name}\n{reason}"),
                    "dialog-error",
                );
            }
            qichi
                .as_mut()
                .set_message(&format!("下载失败：{name} —— {reason}"), false);
            qichi.as_mut().sync_transfers();
        },
    );
}

/// 整批结束：释放占用、给一条总账，然后推进下一批。
///
/// `my_epoch` 与当前代数不一致时说明期间退出过登录：这里什么都别碰，
/// 否则可能把重新登录后的新批次状态清掉。
fn post_finished(batch: u64, cancelled: bool, my_epoch: u64) {
    super::actions::enqueue_ui(
        move |mut qichi: std::pin::Pin<&mut super::qobject::Kichi>| {
            if my_epoch != current_epoch() {
                return;
            }
            let Some((done, failed)) = super::queue::release(batch) else {
                return;
            };
            let message = if cancelled {
                format!("下载已取消（已完成 {done} 个，失败 {failed} 个）")
            } else if failed == 0 {
                format!("下载完成：{done} 个文件")
            } else {
                format!("下载结束：成功 {done} 个，失败 {failed} 个")
            };
            qichi.as_mut().set_message(&message, false);
            qichi.as_mut().advance_downloads();
        },
    );
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
