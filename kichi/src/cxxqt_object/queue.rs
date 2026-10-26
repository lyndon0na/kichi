//! 下载队列的共享状态与后台执行。
//!
//! 与 `downloads.rs` 分工：
//! - `downloads.rs`：QML 能看到的入口（点「⬇」、取消、打开目录）与属性刷新；
//! - 本文件：队列状态、文件夹递归、串行下载线程、进度节流与回投。
//!
//! 后台线程绝不碰 Qt 对象：结果一律经 `actions::enqueue_ui` 回主线程。

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use kichi_core::{EntryKind, FileEntry};

use super::actions::format_size;
use crate::session::download::{Cancel, Progress};
use crate::session::{Failure, Session};

/// 进度回投的最小间隔。
pub(crate) const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);
/// 递归文件夹的最大深度（防环形/异常结构把线程拖死）。
const MAX_DEPTH: usize = 16;

/// 队列里的一个待下载条目。
#[derive(Debug, Clone)]
pub(crate) struct Job {
    pub(crate) id: String,
    /// 落盘用的相对目录（文件夹下载时保留层级；单文件为空）
    pub(crate) sub_dir: Vec<String>,
    pub(crate) name: String,
}

/// 一次生效的批量下载状态（单文件也包成一个批量，收尾逻辑统一）。
#[derive(Debug, Default)]
pub(crate) struct Batch {
    pub(crate) running: bool,
    /// 队列里还没轮到 + 正在下的条目数
    pub(crate) remaining: usize,
    pub(crate) done: usize,
    pub(crate) failed: usize,
    /// 正在下载的条目（QML 显示进度用）
    pub(crate) current: String,
    pub(crate) current_bytes: u64,
    pub(crate) current_total: Option<u64>,
    /// 取消信号：整批共用一个，取消即停
    pub(crate) cancel: Arc<Cancel>,
}

/// 全局批量状态（进程内唯一）。
fn batch() -> &'static Mutex<Batch> {
    static BATCH: OnceLock<Mutex<Batch>> = OnceLock::new();
    BATCH.get_or_init(|| Mutex::new(Batch::default()))
}

/// 在队列状态上加锁（中毒时继续用内部数据：这里只有计数字段）。
pub(crate) fn with_batch<T>(f: impl FnOnce(&mut Batch) -> T) -> T {
    let mut guard = batch().lock().unwrap_or_else(|e| e.into_inner());
    f(&mut guard)
}

/// 登出代数：在途结果用它判断是否已过时。
fn epoch() -> &'static AtomicU64 {
    static EPOCH: OnceLock<AtomicU64> = OnceLock::new();
    EPOCH.get_or_init(|| AtomicU64::new(0))
}

/// 登录态变化时作废在途下载。
pub(crate) fn bump_epoch() {
    epoch().fetch_add(1, Ordering::SeqCst);
}

/// 是否已有下载在跑（同一时刻只有一条队列线程）。
fn busy() -> &'static AtomicBool {
    static FLAG: OnceLock<AtomicBool> = OnceLock::new();
    FLAG.get_or_init(|| AtomicBool::new(false))
}

pub(crate) fn is_busy() -> bool {
    busy().load(Ordering::SeqCst)
}

pub(crate) fn set_busy(value: bool) {
    busy().store(value, Ordering::SeqCst);
}

/// 递归收集文件夹里的文件（保留层级；阻塞）。
pub(crate) fn collect_folder(
    session: &Arc<Session>,
    folder: &FileEntry,
) -> Result<Vec<Job>, Failure> {
    let mut jobs = Vec::new();
    let mut path = vec![folder.name.clone()];
    walk(session, &folder.id, &mut path, &mut jobs, 0)?;
    Ok(jobs)
}

fn walk(
    session: &Arc<Session>,
    folder_id: &str,
    path: &mut Vec<String>,
    jobs: &mut Vec<Job>,
    depth: usize,
) -> Result<(), Failure> {
    if depth > MAX_DEPTH {
        return Err(Failure::Other(format!(
            "目录层级超过 {MAX_DEPTH} 层，为避免异常结构已停止读取"
        )));
    }
    for entry in session.list_dir(Some(folder_id))? {
        match entry.kind {
            EntryKind::Folder => {
                path.push(entry.name.clone());
                walk(session, &entry.id, path, jobs, depth + 1)?;
                path.pop();
            }
            EntryKind::File => jobs.push(Job {
                id: entry.id.clone(),
                sub_dir: path.clone(),
                name: entry.name.clone(),
            }),
        }
    }
    Ok(())
}

/// 队列线程：逐条下载，结果回投主线程。
pub(crate) fn run_queue(session: Arc<Session>, dir: std::path::PathBuf, jobs: Vec<Job>) {
    let my_epoch = epoch().load(Ordering::SeqCst);
    let cancel = with_batch(|state| state.cancel.clone());
    for job in jobs {
        if my_epoch != epoch().load(Ordering::SeqCst) || cancel.is_cancelled() {
            break;
        }
        with_batch(|state| {
            state.current = display_path(&job);
            state.current_bytes = 0;
            state.current_total = None;
        });
        post_progress(job.clone(), 0, None);

        let mut target = dir.clone();
        for part in &job.sub_dir {
            target.push(crate::session::download::safe_file_name(part));
        }
        let throttle = Throttle::default();
        let reporter = job.clone();
        let progress = |bytes: u64, total: Option<u64>| {
            if throttle.should_send(bytes, total) {
                with_batch(|state| {
                    state.current_bytes = bytes;
                    state.current_total = total;
                });
                post_progress(reporter.clone(), bytes, total);
            }
        };
        let outcome = session.download_file(
            &job.id,
            &job.name,
            &target,
            &progress as &Progress<'_>,
            &cancel,
        );
        let cancelled = cancel.is_cancelled();
        with_batch(|state| {
            state.remaining = state.remaining.saturating_sub(1);
            match &outcome {
                Ok(_) => state.done += 1,
                Err(_) => state.failed += 1,
            }
        });
        match outcome {
            Ok(downloaded) => post_saved(&job.name, &downloaded.path),
            Err(_) if cancelled => {
                // 取消不算失败：把刚计入的一次失败退回去，数量对得上总账
                with_batch(|state| {
                    state.failed = state.failed.saturating_sub(1);
                    state.remaining = state.remaining.saturating_add(1);
                });
                break;
            }
            Err(err) => post_failed(&job.name, &err.to_string()),
        }
    }
    post_finished(cancel.is_cancelled());
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

/// 条目 → 界面上显示的路径（`剧集/第 1 集.mkv`）。
fn display_path(job: &Job) -> String {
    if job.sub_dir.is_empty() {
        job.name.clone()
    } else {
        format!("{}/{}", job.sub_dir.join("/"), job.name)
    }
}

// —— 后台线程 → 主线程（不碰 Qt 对象，只入队）——

fn post_progress(job: Job, bytes: u64, total: Option<u64>) {
    super::actions::enqueue_ui(
        move |mut qichi: std::pin::Pin<&mut super::qobject::Kichi>| {
            with_batch(|state| {
                state.current = display_path(&job);
                state.current_bytes = bytes;
                state.current_total = total;
            });
            qichi.as_mut().sync_download_state();
        },
    );
}

fn post_saved(name: &str, path: &std::path::Path) {
    let name = name.to_owned();
    let path = path.display().to_string();
    super::actions::enqueue_ui(
        move |mut qichi: std::pin::Pin<&mut super::qobject::Kichi>| {
            if qichi.notify_enabled {
                super::notifications::show(
                    "下载完成",
                    &format!("{name}\n保存到 {path}"),
                    "folder-download",
                );
            }
            qichi.as_mut().sync_download_state();
        },
    );
}

fn post_failed(name: &str, reason: &str) {
    let name = name.to_owned();
    let reason = reason.to_owned();
    super::actions::enqueue_ui(
        move |mut qichi: std::pin::Pin<&mut super::qobject::Kichi>| {
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
        },
    );
}

/// 整批结束：收尾状态、清忙标志、给一条总账。
fn post_finished(cancelled: bool) {
    super::actions::enqueue_ui(
        move |mut qichi: std::pin::Pin<&mut super::qobject::Kichi>| {
            let (done, failed) = with_batch(|state| {
                state.running = false;
                state.current.clear();
                state.current_bytes = 0;
                state.current_total = None;
                state.remaining = 0;
                (state.done, state.failed)
            });
            set_busy(false);
            let message = if cancelled {
                format!("下载已取消（已完成 {done} 个，失败 {failed} 个）")
            } else if failed == 0 {
                format!("下载完成：{done} 个文件")
            } else {
                format!("下载结束：成功 {done} 个，失败 {failed} 个")
            };
            qichi.as_mut().set_message(&message, false);
            qichi.as_mut().sync_download_state();
        },
    );
}

/// 单条目进度文本（同步到属性时用）。
pub(crate) fn progress_text(current: &str, bytes: u64, total: Option<u64>) -> String {
    if current.is_empty() {
        return String::new();
    }
    match total {
        Some(total) if total > 0 => format!(
            "{current} · {} / {}（{}%）",
            format_size(bytes),
            format_size(total),
            (bytes as f64 / total as f64 * 100.0).clamp(0.0, 100.0) as i32
        ),
        _ => format!("{current} · {}", format_size(bytes)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn throttle_sends_first_then_waits_within_same_percent() {
        let throttle = Throttle::default();
        assert!(throttle.should_send(0, Some(10_000)), "第一次一定发");
        assert!(
            !throttle.should_send(50, Some(10_000)),
            "仍是 0%、未到 250ms：不发"
        );
        assert!(throttle.should_send(500, Some(10_000)), "进到 5% 就发");
    }

    #[test]
    fn throttle_without_total_uses_time_only() {
        let throttle = Throttle::default();
        assert!(throttle.should_send(1, None));
        assert!(!throttle.should_send(2, None), "没有总大小时按时间节流");
    }

    #[test]
    fn display_path_keeps_folder_hierarchy() {
        let nested = Job {
            id: "1".into(),
            sub_dir: vec!["剧集".into(), "第一季".into()],
            name: "第 1 集.mkv".into(),
        };
        assert_eq!(display_path(&nested), "剧集/第一季/第 1 集.mkv");
        let single = Job {
            sub_dir: Vec::new(),
            ..nested
        };
        assert_eq!(display_path(&single), "第 1 集.mkv");
    }

    #[test]
    fn progress_text_shows_percent_only_with_total() {
        assert_eq!(progress_text("", 0, None), "", "没有当前文件就不显示");
        assert!(progress_text("a.mkv", 50, Some(100)).contains("50%"));
        assert!(!progress_text("a.mkv", 50, None).contains('%'));
    }
}
