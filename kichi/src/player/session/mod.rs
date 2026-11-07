//! 播放会话登记表 + 监控线程 + 退出收尾。
//!
//! 每条播放会话由 [`start`] 创建，事件经 `notify` 回投给界面层
//! （调用方负责切回主线程）。同一条会话会先发 [`StartEvent::Ready`]，
//! 播放器真正退出后再发 [`StartEvent::Stopped`]。登记表只做三件事：
//! 取消、统计、退出时全部关掉。
//!
//! 生命周期：
//! - mpv：spawn idle + IPC → observe + loadfile → 等窗口 → Ready → 等进程退出；
//! - VLC：spawn 私有清单 + oldrc → 轮询 `is_playing` → Ready → 轮询进度 / 等退出；
//! - 取消加载统一发 `quit`，800 ms 还不退就 `kill`，只回投 `Cancelled`；
//! - 会话在播放器退出后从登记表移除（正常播完同样移除）。
//!
//! 缩到托盘**不算**退出：已经在播的 mpv 继续；真退出时调 [`shutdown_all`]。
//!
//! 按职责拆分：本文件是登记表、入口与进程收尾，[`mpv`] / [`vlc`] 各自
//! 负责自己的监控循环。

use std::collections::HashMap;
use std::process::Child;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier, Mutex, OnceLock};
use std::time::{Duration, Instant};

use kichi_core::PlaybackProgress;

use super::ipc;
use super::subtitles::{LateSubtitles, SubtitleAssets};
use super::{launch, mask_url, PlayerConfig, PlayerProtocol};
use crate::session::download::Cancel;

mod mpv;
#[cfg(test)]
mod tests;
mod vlc;

use mpv::run_monitor;
use vlc::start_vlc;

/// 取消后给 mpv 的退出宽限；超过就强杀。
const QUIT_GRACE: Duration = Duration::from_millis(800);
/// kichi 退出时给全部 mpv 的总宽限。
const SHUTDOWN_GRACE: Duration = Duration::from_secs(1);
/// 监控线程的轮询粒度（检查取消标志 / 进程是否已退）。
const POLL: Duration = Duration::from_millis(200);
/// 播放器进程退出后，再等 IPC 把 `end-file(eof)` 等尾部事件送完的宽限。
const EXIT_EVENT_GRACE: Duration = Duration::from_millis(500);
/// VLC RC 没有事件推送，按这个间隔主动读取位置与时长。
#[cfg(not(test))]
const VLC_POLL: Duration = Duration::from_millis(1000);
#[cfg(test)]
const VLC_POLL: Duration = Duration::from_millis(100);
/// 一次播放结束的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    /// 播放器自然播到文件尾。
    Eof,
    /// 用户关闭窗口 / 主动停止播放。
    Stopped,
    /// 无法判断（通用播放器、异常退出等）。
    Unknown,
}

/// 一次播放的结果（回投给界面层）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartEvent {
    /// 播放器已就绪（mpv 窗口 / 音频事件，或 VLC `is_playing=1`）。
    ///
    /// `subtitle_note` 是自动字幕没能挂上去的原因；没有字幕 / 成功时为 `None`。
    Ready { subtitle_note: Option<String> },
    /// 播放中定期回投的官方历史进度。
    Progress { progress: PlaybackProgress },
    /// 播放器进程已退出；只有此前真的发过 `Ready` 才会出现。
    Stopped {
        progress: Option<PlaybackProgress>,
        end: EndReason,
    },
    /// 启动失败 / 播放器在出窗口前退出。文案已含文件名，界面直接显示
    Failed(String),
    /// 用户取消（界面不写任何文案）
    Cancelled,
}

/// 播放器的可选资产与续播位置。
#[derive(Default)]
pub struct PlaybackExtras {
    pub subtitles: Option<SubtitleAssets>,
    pub late: Option<LateSubtitles>,
    pub resume: Option<PlaybackProgress>,
}

/// 一条登记在案的播放会话。
struct Session {
    cancel: Arc<Cancel>,
    /// 监控线程持有的进程句柄（退出收尾时也要能拿到）
    child: Arc<Mutex<Option<Child>>>,
    /// 这条会话产生的临时文件（socket / VLC 清单），退出收尾时兜底删除
    cleanup: Vec<std::path::PathBuf>,
}

fn sessions() -> &'static Mutex<HashMap<u64, Session>> {
    static SESSIONS: OnceLock<Mutex<HashMap<u64, Session>>> = OnceLock::new();
    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn next_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

fn with_sessions<T>(f: impl FnOnce(&mut HashMap<u64, Session>) -> T) -> T {
    let mut guard = sessions().lock().unwrap_or_else(|e| e.into_inner());
    f(&mut guard)
}

/// 启动一次播放：mpv → idle + IPC；VLC → oldrc；其它 → 旧清单方式。
///
/// 事件通过 `notify` 回投（调用方负责切回主线程）。一次会话最多回投
/// `Ready → Stopped`，或 `Failed` / `Cancelled`。返回会话 id（通用降级播放器
/// 固定为 `0`，它没有可取消的会话）；启动失败（spawn 失败 / 线程起不来）返回 `Err`，
/// 由调用方放进提示条。
pub fn start(
    config: &PlayerConfig,
    url: &str,
    title: &str,
    cancel: Arc<Cancel>,
    extras: PlaybackExtras,
    notify: impl Fn(StartEvent) + Send + Sync + 'static,
) -> Result<u64, String> {
    if url.trim().is_empty() {
        return Err("播放链接为空".to_owned());
    }
    if config.protocol() != PlayerProtocol::Generic && config.uses_input_placeholder() {
        return Err(
            "mpv / VLC 的播放链接由控制接口传入，参数里不能使用 {url} 或 {playlist}".to_owned(),
        );
    }
    if !config.is_mpv() {
        if config.protocol() == PlayerProtocol::Vlc {
            return start_vlc(config, url, title, cancel, extras, notify);
        }
        // 非 mpv / VLC：spawn 成功即视为开播（没有窗口探测，也没法挂字幕）
        drop(extras);
        let notify = Arc::new(notify);
        let ready = Arc::new(Barrier::new(2));
        let exit_notify = notify.clone();
        let exit_ready = ready.clone();
        launch::spawn(config, url, title, move || {
            // 极端情况下播放器可能在 Ready 回投前就退出。先对齐一次，
            // 保证界面总是收到 Ready → Stopped，而不是反过来留下永久标记。
            exit_ready.wait();
            exit_notify(StartEvent::Stopped {
                progress: None,
                end: EndReason::Unknown,
            });
        })?;
        notify(StartEvent::Ready {
            subtitle_note: None,
        });
        ready.wait();
        return Ok(0);
    }

    let notify = Arc::new(notify);

    super::ensure_play_dir()?;
    let socket = ipc::socket_path();
    // 上一次异常退出可能留下同名文件（计数器已保证不重复，防御性删除）
    let _ = std::fs::remove_file(&socket);

    let mut command = ipc::build_command(config, &socket, title);
    let child = command
        .spawn()
        .map_err(|err| format!("启动播放器 {} 失败：{err}", config.program()))?;
    eprintln!(
        "[kichi] 启动 mpv 播放「{title}」→ {}（socket {}）",
        mask_url(url),
        socket.display()
    );

    let id = next_id();
    let child = Arc::new(Mutex::new(Some(child)));
    with_sessions(|map| {
        map.insert(
            id,
            Session {
                cancel: cancel.clone(),
                child: child.clone(),
                cleanup: vec![socket.clone()],
            },
        );
    });

    let program = config.program().to_owned();
    let title = title.to_owned();
    let url = url.to_owned();
    let monitor_socket = socket.clone();
    let monitor_child = child.clone();
    let monitor_notify = notify.clone();
    let PlaybackExtras {
        subtitles,
        late,
        resume,
    } = extras;
    let spawned = std::thread::Builder::new()
        .name("kichi-mpv".into())
        .spawn(move || {
            run_monitor(
                &monitor_socket,
                &monitor_child,
                &cancel,
                &url,
                &title,
                &program,
                subtitles,
                late,
                resume,
                move |event| monitor_notify(event),
            );
            // 进程已由监控线程收尾（或被强杀）：从登记表移除，
            // 别让退出收尾再等一次
            with_sessions(|map| map.remove(&id));
            let _ = std::fs::remove_file(&monitor_socket);
        });
    if let Err(err) = spawned {
        // 线程起不来必须显式收尾：关掉刚 spawn 的 mpv，并注销这条会话
        // （踩坑第 16 条：不能只打 stderr）
        with_sessions(|map| map.remove(&id));
        let _ = std::fs::remove_file(&socket);
        kill_child(&child);
        return Err(format!("启动播放监控线程失败：{err}"));
    }
    Ok(id)
}

struct MonitorOutcome {
    was_ready: bool,
    final_progress: Option<PlaybackProgress>,
    end: EndReason,
}

/// 取消一条会话（任意线程可调）。只置标志，真正关闭播放器由监控线程做。
///
/// 界面侧的取消走 `Cancel` 标志本身（提示条持有同一个 `Arc`）；
/// 这个入口留给「按会话 id 取消」的调用方与测试
/// （`#[allow(dead_code)]`：生产路径暂时只用标志取消，保留它是登记表的完整接口）。
#[allow(dead_code)]
pub fn cancel(id: u64) {
    with_sessions(|map| {
        if let Some(session) = map.get(&id) {
            session.cancel.cancel();
        }
    });
}

/// kichi 真退出：逐个 `quit` → 1 秒宽限 → 强杀。可重复调用。
pub fn shutdown_all() {
    // 先让所有监控线程知道「不用再等窗口了」
    let pending = with_sessions(|map| {
        map.values()
            .map(|session| {
                session.cancel.cancel();
            })
            .count()
    });
    if pending == 0 {
        return;
    }
    // 没有 socket 句柄可写 quit（监控线程自己持有连接），这里等它们收尾；
    // 超时后统一强杀，保证 kichi 退出后不留孤儿
    let deadline = Instant::now() + SHUTDOWN_GRACE;
    while Instant::now() < deadline && active_count() > 0 {
        std::thread::sleep(POLL);
    }
    let leftovers =
        with_sessions(|map| map.drain().map(|(_, session)| session).collect::<Vec<_>>());
    for session in leftovers {
        kill_child(&session.child);
        // 进程没了 socket / 清单就是死文件：在这里删，别指望监控线程跑到最后一行
        for path in session.cleanup {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// 当前还登记着的播放会话数（测试与日志用）。
pub fn active_count() -> usize {
    with_sessions(|map| map.len())
}

/// 等播放器自己退出并回收进程；不长时间持锁（取消路径不能被它挡住）。
fn wait_for_exit(child: &Arc<Mutex<Option<Child>>>) {
    while !child_exited(child) {
        std::thread::sleep(POLL);
    }
}

/// 进程是否已经退出（退出了就顺手回收句柄）。不长时间持锁。
fn child_exited(child: &Arc<Mutex<Option<Child>>>) -> bool {
    let mut guard = child.lock().unwrap_or_else(|e| e.into_inner());
    match guard.as_mut() {
        Some(process) => match process.try_wait() {
            Ok(Some(_)) => {
                *guard = None;
                true
            }
            Ok(None) => false,
            Err(err) => {
                eprintln!("[kichi] 查询播放器状态失败：{err}");
                *guard = None;
                true
            }
        },
        None => true,
    }
}

/// 给 RC 断流之后的播放器一点时间退出：假 VLC / 参数错误可能在最后一行输出
/// 和进程回收之间有一个很小的窗口，不能立刻误报成「已交给播放器」。
fn child_exits_within(child: &Arc<Mutex<Option<Child>>>, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if child_exited(child) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// 强杀进程并回收（进程已经退出时什么都不做）。
fn kill_child(child: &Arc<Mutex<Option<Child>>>) {
    let mut guard = child.lock().unwrap_or_else(|e| e.into_inner());
    let Some(mut process) = guard.take() else {
        return;
    };
    if let Err(err) = process.kill() {
        eprintln!("[kichi] 关闭播放器失败：{err}");
    }
    let _ = process.wait();
}
