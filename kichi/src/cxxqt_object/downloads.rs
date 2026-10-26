//! 下载到本地：任务队列、进度上报、完成通知。
//!
//! 队列节奏：
//! - **一次只下一个文件**（串行）。并行会互相抢带宽，官方也会视为压力；
//!   文件夹下载本来就是一个接一个排队，串行最能反映真实进度。
//! - 进度由下载线程经主线程队列回投（`pump()` 消费），后台线程不碰任何 Qt 对象。
//! - 进度节流：每 250ms 或每进 1% 才回投一次，避免长文件把主线程刷爆。
//!
//! 退出登录：`epoch()` 自增，在途结果与队列一起作废。

use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;
use kichi_core::{EntryKind, FileEntry};

use super::actions::{format_size, run_in_background};
use super::qobject;
use crate::session::download::{Cancel, Progress};
use crate::session::{Failure, Session};

/// 进度回投的最小间隔。
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);
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

fn with_batch<T>(f: impl FnOnce(&mut Batch) -> T) -> T {
    let mut guard = batch().lock().unwrap_or_else(|e| e.into_inner());
    f(&mut guard)
}

/// 登出代数：在途结果用它判断是否已过时。
fn epoch() -> &'static AtomicU64 {
    static EPOCH: OnceLock<AtomicU64> = OnceLock::new();
    EPOCH.get_or_init(|| AtomicU64::new(0))
}

/// 是否已有下载在跑（保证同一时刻只有一条队列线程）。
fn busy() -> &'static AtomicBool {
    static FLAG: OnceLock<AtomicBool> = OnceLock::new();
    FLAG.get_or_init(|| AtomicBool::new(false))
}

impl qobject::Kichi {
    /// 下载一个条目（文件直接下；文件夹递归排队）。界面点「⬇」走这里。
    pub fn download_entry(mut self: Pin<&mut Self>, id: QString) {
        let id = id.to_string();
        let Some(entry) = self.entries.iter().find(|e| e.id == id).cloned() else {
            self.as_mut()
                .set_message("该条目已不在当前列表中，请刷新后重试", false);
            return;
        };
        if busy().load(Ordering::SeqCst) {
            self.as_mut().set_message(
                "已有下载在进行，等它结束再试（当前版本一次只下一个）",
                false,
            );
            return;
        }
        let dir = self.settings.download_dir_path();
        if entry.kind == EntryKind::Folder {
            let session = self.backend.session.clone();
            self.as_mut()
                .set_message(&format!("正在读取「{}」的目录结构……", entry.name), true);
            run_in_background(
                move || collect_folder(&session, &entry),
                move |mut qichi, result| match result {
                    Ok(jobs) => qichi.as_mut().start_batch(dir, jobs),
                    Err(err) => qichi.as_mut().on_download_failure(err),
                },
            );
            return;
        }
        let jobs = vec![Job {
            id: entry.id.clone(),
            sub_dir: Vec::new(),
            name: entry.name.clone(),
        }];
        self.as_mut().start_batch(dir, jobs);
    }

    /// 取消当前下载（整批停掉；已下完的文件保留）。
    pub fn cancel_downloads(mut self: Pin<&mut Self>) {
        with_batch(|state| {
            state.cancel.cancel();
            state.running = false;
        });
        self.as_mut().set_message("正在取消下载……", false);
        self.as_mut().sync_download_state();
    }

    /// 在文件管理器里打开下载目录（没建就顺手建好）。
    pub fn open_download_dir(&self) {
        let dir = self.settings.download_dir_path();
        if let Err(err) = std::fs::create_dir_all(&dir) {
            eprintln!("[kichi] 创建下载目录失败：{err}");
        }
        let spawned = std::process::Command::new("xdg-open")
            .arg(&dir)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        if let Err(err) = spawned {
            eprintln!("[kichi] 打开下载目录失败：{err}");
        }
    }

    /// 启动一批下载（主线程调用；队列线程从这里接手）。
    pub(crate) fn start_batch(mut self: Pin<&mut Self>, dir: std::path::PathBuf, jobs: Vec<Job>) {
        if jobs.is_empty() {
            self.as_mut().set_message("这个位置没有可下载的文件", false);
            return;
        }
        let total = jobs.len();
        {
            let cancel = Arc::new(Cancel::default());
            with_batch(|state| {
                *state = Batch {
                    running: true,
                    remaining: total,
                    cancel,
                    ..Batch::default()
                };
            });
        }
        busy().store(true, Ordering::SeqCst);
        self.as_mut()
            .set_message(&format!("开始下载 {total} 个文件……"), false);
        self.as_mut().sync_download_state();

        let session = self.backend.session.clone();
        let spawned = std::thread::Builder::new()
            .name("kichi-dl".into())
            .spawn(move || run_queue(session, dir, jobs));
        if let Err(err) = spawned {
            busy().store(false, Ordering::SeqCst);
            with_batch(|state| state.running = false);
            eprintln!("[kichi] 无法创建下载线程：{err}");
            self.as_mut()
                .set_message("无法创建下载线程，下载未开始", false);
            self.as_mut().sync_download_state();
        }
    }

    /// 把队列状态同步到 QML 属性（主线程调用）。
    pub(crate) fn sync_download_state(mut self: Pin<&mut Self>) {
        let (current, bytes, total, remaining, done, failed, running) = with_batch(|state| {
            (
                state.current.clone(),
                state.current_bytes,
                state.current_total,
                state.remaining,
                state.done,
                state.failed,
                state.running,
            )
        });
        let text = if current.is_empty() {
            String::new()
        } else {
            match total {
                Some(total) if total > 0 => format!(
                    "{current} · {} / {}（{}%）",
                    format_size(bytes),
                    format_size(total),
                    (bytes as f64 / total as f64 * 100.0).clamp(0.0, 100.0) as i32
                ),
                _ => format!("{current} · {}", format_size(bytes)),
            }
        };
        {
            let mut rust = self.as_mut().rust_mut();
            rust.download_current = QString::from(text.as_str());
            rust.download_running = running;
            rust.download_remaining = i32::try_from(remaining).unwrap_or(i32::MAX);
            rust.download_done = i32::try_from(done).unwrap_or(i32::MAX);
            rust.download_failed = i32::try_from(failed).unwrap_or(i32::MAX);
        }
        self.as_mut().notify();
    }

    /// 下载相关错误的统一提示（令牌失效仍走登录失效分支）。
    fn on_download_failure(mut self: Pin<&mut Self>, failure: Failure) {
        if matches!(failure, Failure::Auth(_)) {
            self.as_mut().on_failure(failure);
            return;
        }
        self.as_mut().set_message(&failure.to_string(), false);
    }

    /// 登出时作废下载：取消在途任务并清空状态。
    pub(crate) fn reset_downloads(mut self: Pin<&mut Self>) {
        epoch().fetch_add(1, Ordering::SeqCst);
        with_batch(|state| {
            state.cancel.cancel();
            *state = Batch::default();
        });
        busy().store(false, Ordering::SeqCst);
        self.as_mut().sync_download_state();
    }
}

/// 递归收集文件夹里的文件（保留层级；阻塞）。
fn collect_folder(session: &Arc<Session>, folder: &FileEntry) -> Result<Vec<Job>, Failure> {
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
fn run_queue(session: Arc<Session>, dir: std::path::PathBuf, jobs: Vec<Job>) {
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
            Err(_err) if cancelled => {
                // 取消不算失败：把刚计入的失败减回去
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
/// 只被下载线程使用，但进度回调要求 `Send`（且这里用原子量，`Sync` 也成立），
/// 免得将来挪到别的线程就编译不过。
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
        let due = match *last {
            Some(previous) => now.duration_since(previous) >= PROGRESS_INTERVAL,
            None => true,
        };
        let previous_percent = self.last_percent.load(Ordering::Relaxed);
        // 首次发送只把定时器开起来：这次不发，下一秒才是真正的「距上次多久」
        let first = last.is_none();
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
    super::actions::enqueue_ui(move |mut qichi: Pin<&mut qobject::Kichi>| {
        with_batch(|state| {
            state.current = display_path(&job);
            state.current_bytes = bytes;
            state.current_total = total;
        });
        qichi.as_mut().sync_download_state();
    });
}

fn post_saved(name: &str, path: &std::path::Path) {
    let name = name.to_owned();
    let path = path.display().to_string();
    super::actions::enqueue_ui(move |mut qichi: Pin<&mut qobject::Kichi>| {
        if qichi.notify_enabled {
            super::notifications::show(
                "下载完成",
                &format!("{name}\n保存到 {path}"),
                "folder-download",
            );
        }
        qichi.as_mut().sync_download_state();
    });
}

fn post_failed(name: &str, reason: &str) {
    let name = name.to_owned();
    let reason = reason.to_owned();
    super::actions::enqueue_ui(move |mut qichi: Pin<&mut qobject::Kichi>| {
        if qichi.notify_enabled {
            super::notifications::show("下载失败", &format!("{name}\n{reason}"), "dialog-error");
        }
        qichi
            .as_mut()
            .set_message(&format!("下载失败：{name} —— {reason}"), false);
    });
}

/// 整批结束：收尾状态、清忙标志、给一条总账。
fn post_finished(cancelled: bool) {
    super::actions::enqueue_ui(move |mut qichi: Pin<&mut qobject::Kichi>| {
        let (done, failed) = with_batch(|state| {
            state.running = false;
            state.current.clear();
            state.current_bytes = 0;
            state.current_total = None;
            state.remaining = 0;
            (state.done, state.failed)
        });
        busy().store(false, Ordering::SeqCst);
        let message = if cancelled {
            format!("下载已取消（已完成 {done} 个，失败 {failed} 个）")
        } else if failed == 0 {
            format!("下载完成：{done} 个文件")
        } else {
            format!("下载结束：成功 {done} 个，失败 {failed} 个")
        };
        qichi.as_mut().set_message(&message, false);
        qichi.as_mut().sync_download_state();
    });
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
}
