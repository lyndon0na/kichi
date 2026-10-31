//! 播放会话登记表 + 监控线程 + 退出收尾。
//!
//! 每条播放会话由 [`start`] 创建，事件经 `notify` 回投给界面层
//! （调用方负责切回主线程）。同一条会话会先发 [`StartEvent::Ready`]，
//! 播放器真正退出后再发 [`StartEvent::Stopped`]。登记表只做三件事：
//! 取消、统计、退出时全部关掉。
//!
//! 生命周期（mpv 路径）：
//! 1. spawn mpv（`--idle=yes` + IPC socket）；
//! 2. 连 socket → `observe_property` + `loadfile`；
//! 3. 等窗口出现（[`ipc::watch`]）；
//! 4. 就绪 → 回投 `Ready`，再 `set_property idle no`（播完自动退出）；
//! 5. 在这条线程上等 mpv 退出；正常播完或 kichi 主动关闭都回投 `Stopped`；
//! 6. 取消加载 → 发 `quit`，800 ms 还不退就 `kill`，只回投 `Cancelled`；
//! 7. 会话在 mpv 退出后从登记表移除（正常播完同样移除）。
//!
//! 缩到托盘**不算**退出：已经在播的 mpv 继续；真退出时调 [`shutdown_all`]。

use std::collections::HashMap;
use std::process::Child;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier, Mutex, OnceLock};
use std::time::{Duration, Instant};

use kichi_core::PlaybackProgress;

use super::ipc::{self, StartOutcome};
use super::progress::ProgressTracker;
use super::subtitles::{self, LateSubtitles, SubtitleAssets};
use super::{launch, mask_url, PlayerConfig};
use crate::session::download::Cancel;

/// 取消后给 mpv 的退出宽限；超过就强杀。
const QUIT_GRACE: Duration = Duration::from_millis(800);
/// kichi 退出时给全部 mpv 的总宽限。
const SHUTDOWN_GRACE: Duration = Duration::from_secs(1);
/// 监控线程的轮询粒度（检查取消标志 / 进程是否已退）。
const POLL: Duration = Duration::from_millis(200);
/// 一次播放的结果（回投给界面层）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartEvent {
    /// 窗口已出现（或纯音频已开始解码）。
    ///
    /// `subtitle_note` 是自动字幕没能挂上去的原因；没有字幕 / 成功时为 `None`。
    Ready { subtitle_note: Option<String> },
    /// 播放中定期回投的官方历史进度。
    Progress { progress: PlaybackProgress },
    /// 播放器进程已退出；只有此前真的发过 `Ready` 才会出现。
    Stopped { progress: Option<PlaybackProgress> },
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
    /// 这条会话的 IPC socket：退出收尾时由调用方删除，不依赖监控线程跑到最后
    socket: Option<std::path::PathBuf>,
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

/// 启动一次播放：mpv → idle + IPC；非 mpv → 旧清单方式。
///
/// 事件通过 `notify` 回投（调用方负责切回主线程）。一次会话最多回投
/// `Ready → Stopped`，或 `Failed` / `Cancelled`。返回会话 id（非 mpv 固定为
/// `0`，它没有可取消的会话）；启动失败（spawn 失败 / 线程起不来）返回 `Err`，
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
    if !config.is_mpv() {
        // 非 mpv：spawn 成功即视为开播（没有窗口探测，也没法挂字幕）
        drop(extras);
        let notify = Arc::new(notify);
        let ready = Arc::new(Barrier::new(2));
        let exit_notify = notify.clone();
        let exit_ready = ready.clone();
        launch::spawn(config, url, title, move || {
            // 极端情况下播放器可能在 Ready 回投前就退出。先对齐一次，
            // 保证界面总是收到 Ready → Stopped，而不是反过来留下永久标记。
            exit_ready.wait();
            exit_notify(StartEvent::Stopped { progress: None });
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
                socket: Some(socket.clone()),
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

/// 监控线程主体：连接 → observe + loadfile → 等就绪 → 收尾。
// 参数都是监控线程的固定上下文（socket / 进程 / 取消 / 直链 / 标题 / 程序名 /
// 字幕资产 / 回投），拆成结构体只会多一层解包，没有可读性收益。
#[allow(clippy::too_many_arguments)]
fn run_monitor(
    socket: &std::path::Path,
    child: &Arc<Mutex<Option<Child>>>,
    cancel: &Cancel,
    url: &str,
    title: &str,
    program: &str,
    subtitles: Option<SubtitleAssets>,
    late: Option<LateSubtitles>,
    resume: Option<PlaybackProgress>,
    emit: impl Fn(StartEvent),
) {
    let outcome = run_monitor_before_exit(
        socket, child, cancel, url, title, program, subtitles, late, resume, &emit,
    );
    if outcome.was_ready {
        emit(StartEvent::Stopped {
            progress: outcome.final_progress,
        });
    }
}

struct MonitorOutcome {
    was_ready: bool,
    final_progress: Option<PlaybackProgress>,
}

/// 跑到播放器就绪并等它退出；结果包含是否发过 `Ready` 与最后一次有效进度。
///
/// 单独拆出来是为了保证所有开播后的退出路径都恰好补发一次 `Stopped`，
/// 包括 IPC 不可观测、正常播完和 kichi 退出时主动关闭。
#[allow(clippy::too_many_arguments)]
fn run_monitor_before_exit(
    socket: &std::path::Path,
    child: &Arc<Mutex<Option<Child>>>,
    cancel: &Cancel,
    url: &str,
    title: &str,
    program: &str,
    subtitles: Option<SubtitleAssets>,
    late: Option<LateSubtitles>,
    resume: Option<PlaybackProgress>,
    emit: &impl Fn(StartEvent),
) -> MonitorOutcome {
    let no_status_note = || {
        subtitles
            .as_ref()
            .filter(|assets| !assets.is_empty())
            .map(|_| "当前播放器无法确认播放状态，字幕未加载".to_owned())
    };
    let Some(mut conn) = ipc::connect(socket, cancel) else {
        if cancel.is_cancelled() {
            kill_child(child);
            emit(StartEvent::Cancelled);
            return MonitorOutcome {
                was_ready: false,
                final_progress: None,
            };
        }
        // 连不上：如果进程已经退出（flatpak 起不来、参数被拒），那是真失败；
        // 进程还活着才按「已交给播放器」处理，但不提供取消（见 ADR 0007 的代价）
        if child_exited(child) {
            emit(StartEvent::Failed(format!(
                "mpv 未能播放「{title}」：播放器已退出（无法连接 IPC）"
            )));
            return MonitorOutcome {
                was_ready: false,
                final_progress: None,
            };
        }
        eprintln!("[kichi] 连不上 mpv 的 IPC，无法观测播放状态（{program}）");
        let no_ipc_note = resume
            .is_some()
            .then(|| "无法连接播放器，未能恢复上次进度".to_owned());
        emit(StartEvent::Ready {
            subtitle_note: join_notes(no_status_note(), no_ipc_note),
        });
        wait_for_exit(child);
        return MonitorOutcome {
            was_ready: true,
            final_progress: None,
        };
    };
    if let Err(err) = conn
        .observe_start()
        .and_then(|()| conn.observe_log())
        .and_then(|()| conn.loadfile(url))
    {
        eprintln!("[kichi] 向 mpv 发送播放命令失败：{err}");
        let _ = conn.quit();
        kill_child(child);
        emit(StartEvent::Failed(format!(
            "mpv 未能播放「{title}」：无法与播放器通信（IPC 写入失败）"
        )));
        return MonitorOutcome {
            was_ready: false,
            final_progress: None,
        };
    }

    // 链接打不开时用**同一个**直链重试：PikPak 的签名直链偶发第一次被 CDN 拒
    // （403），立刻再来一次就能播（实测复现，见 ADR 0007）。
    let mut attempt = 0usize;
    // `attach_ok`：只有真正看到窗口才挂字幕；`Unknown` 也走 `None`，但文件
    // 可能还没加载，`sub-add` 会报错。
    let (reason, attach_ok) = loop {
        match ipc::watch(&mut conn, cancel) {
            StartOutcome::Ready => break (None, true),
            StartOutcome::Cancelled => {
                shut_down(&mut conn, child);
                emit(StartEvent::Cancelled);
                return MonitorOutcome {
                    was_ready: false,
                    final_progress: None,
                };
            }
            StartOutcome::Failed(reason) => break (Some(reason), false),
            StartOutcome::LoadFailed(reason) => {
                if attempt >= ipc::LOAD_RETRIES {
                    break (Some(reason), false);
                }
                let delay = ipc::retry_delay(attempt);
                attempt += 1;
                eprintln!(
                    "[kichi] 播放「{title}」失败（第 {attempt} 次），{delay:?} 后用同一链接重试"
                );
                if !crate::session::download::sleep_cancellable(cancel, delay) {
                    shut_down(&mut conn, child);
                    emit(StartEvent::Cancelled);
                    return MonitorOutcome {
                        was_ready: false,
                        final_progress: None,
                    };
                }
                if let Err(err) = conn.loadfile(url) {
                    eprintln!("[kichi] 重试播放失败（IPC 写入）：{err}");
                    break (Some("无法与播放器通信（IPC 写入失败）".to_owned()), false);
                }
                continue;
            }
            StartOutcome::Unknown => {
                eprintln!("[kichi] 无法观测 mpv 的窗口状态，按已开播处理（{program}）");
                break (None, false);
            }
        }
    };

    match reason {
        None => {
            // 挂字幕前先查取消：别把最多 500 ms 的回执等待加在取消路径上
            if cancel.is_cancelled() {
                shut_down(&mut conn, child);
                emit(StartEvent::Cancelled);
                return MonitorOutcome {
                    was_ready: false,
                    final_progress: None,
                };
            }
            // 请求 id 与「有没有 select 过」跨预挂 / 播放中补挂共用
            let mut next_sub_id = ipc::SUB_ADD_REQ_BASE;
            let mut selected = false;
            let subtitle_note = if attach_ok {
                subtitles::attach(
                    &mut conn,
                    subtitles.as_ref(),
                    &mut next_sub_id,
                    &mut selected,
                )
            } else {
                no_status_note()
            };
            if let Some(progress) = resume {
                if let Err(err) = conn.seek_absolute(progress.seconds) {
                    eprintln!("[kichi] 恢复播放进度失败（只记日志）：{err}");
                }
            }
            // 先收起提示条：窗口已经出现，后面的 idle / 等退出都不该让用户等
            emit(StartEvent::Ready { subtitle_note });
            // 播完自动退出；失败只记一句，不影响「已经开播」这个结论
            if let Err(err) = conn.disable_idle() {
                eprintln!("[kichi] 关闭 mpv 的 idle 失败：{err}");
            }
            // 已开播：这里的取消只来自「退出 kichi」（shutdown_all），
            // 收到就请 mpv 退出，超时强杀。预等没下完的字幕在这期间补挂。
            let mut late_held: Vec<SubtitleAssets> = Vec::new();
            let hooks = PlayingHooks {
                late: late.as_ref(),
                late_held: &mut late_held,
                next_sub_id: &mut next_sub_id,
                selected: &mut selected,
                fallback_duration: resume.map(|progress| progress.duration),
                emit,
            };
            let final_progress = wait_playing(&mut conn, child, cancel, hooks);
            MonitorOutcome {
                was_ready: true,
                final_progress,
            }
        }
        Some(reason) => {
            // HTTP 状态码只在 mpv 日志里（"http: HTTP error 403 Forbidden"）
            let detail = match conn.last_http_error.take() {
                Some(http) if !reason.contains(&http) => format!("{reason}，{http}"),
                _ => reason,
            };
            eprintln!("[kichi] 播放「{title}」最终失败（已重试 {attempt} 次）：{detail}");
            let _ = conn.quit();
            kill_child(child);
            emit(StartEvent::Failed(format!(
                "mpv 未能播放「{title}」：{detail}"
            )));
            MonitorOutcome {
                was_ready: false,
                final_progress: None,
            }
        }
    }
}

fn join_notes(first: Option<String>, second: Option<String>) -> Option<String> {
    match (first, second) {
        (Some(first), Some(second)) => Some(format!("{first}；{second}")),
        (first, second) => first.or(second),
    }
}

/// 已经开播：等 mpv 自己播完退出；若期间收到取消（kichi 真退出），
/// 发 `quit` 并宽限，仍不退出就强杀。
///
/// 预等没下完的字幕在这期间从共享槽里取出来补挂：取到的资产必须留到播放结束
/// （`late_held`），否则临时目录会提前删掉、mpv 读到半路文件消失。
struct PlayingHooks<'a, F> {
    late: Option<&'a LateSubtitles>,
    late_held: &'a mut Vec<SubtitleAssets>,
    next_sub_id: &'a mut u64,
    selected: &'a mut bool,
    fallback_duration: Option<u64>,
    emit: &'a F,
}

fn wait_playing(
    conn: &mut ipc::Connection,
    child: &Arc<Mutex<Option<Child>>>,
    cancel: &Cancel,
    hooks: PlayingHooks<'_, impl Fn(StartEvent)>,
) -> Option<PlaybackProgress> {
    let PlayingHooks {
        late,
        late_held,
        next_sub_id,
        selected,
        fallback_duration,
        emit,
    } = hooks;
    let mut tracker = ProgressTracker::new(
        conn.latest_time_pos,
        conn.latest_duration,
        fallback_duration,
        Instant::now(),
    );
    loop {
        if cancel.is_cancelled() {
            shut_down(conn, child);
            return tracker.final_progress();
        }
        if child_exited(child) {
            return tracker.final_progress();
        }
        if let Some(late) = late {
            if let Some(assets) = subtitles::take_slot(&late.slot) {
                if !assets.is_empty() {
                    match subtitles::attach(conn, Some(&assets), next_sub_id, selected) {
                        None => {
                            eprintln!("[kichi] 字幕：播放中补挂 {} 条", assets.tracks().len());
                        }
                        Some(note) => eprintln!("[kichi] 字幕：播放中补挂失败：{note}"),
                    }
                }
                // attach 等回执时会读走属性事件；把连接里缓存的最新值补回跟踪器。
                tracker.update_values(conn.latest_time_pos, conn.latest_duration);
                late_held.push(assets);
            }
        }
        match conn.read_playback_event() {
            Ok(ipc::PlaybackRead::Message(value)) => tracker.observe(&value),
            Ok(ipc::PlaybackRead::Timeout) => {}
            Ok(ipc::PlaybackRead::Closed) => {
                // socket 已断时进程通常马上退出；下一轮统一收尾。
                std::thread::sleep(POLL);
            }
            Err(err) => {
                eprintln!("[kichi] 读取播放器进度失败：{err}");
                std::thread::sleep(POLL);
            }
        }
        if let Some(progress) = tracker.maybe_emit(Instant::now()) {
            emit(StartEvent::Progress { progress });
        }
    }
}

/// 取消一条会话（任意线程可调）。只置标志，真正关 mpv 由监控线程做。
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
        // 进程没了 socket 就是死文件：在这里删，别指望监控线程跑到最后一行
        if let Some(socket) = session.socket {
            let _ = std::fs::remove_file(socket);
        }
    }
}

/// 当前还登记着的播放会话数（测试与日志用）。
pub fn active_count() -> usize {
    with_sessions(|map| map.len())
}

/// 等 mpv 自己退出并回收进程；不长时间持锁（取消路径不能被它挡住）。
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

/// 取消收尾：先请它退出，宽限内不退出就强杀。
fn shut_down(conn: &mut ipc::Connection, child: &Arc<Mutex<Option<Child>>>) {
    if let Err(err) = conn.quit() {
        eprintln!("[kichi] 请求 mpv 退出失败：{err}");
    }
    let deadline = Instant::now() + QUIT_GRACE;
    while !child_exited(child) {
        if Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    kill_child(child);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 登记表是进程级全局状态，测试之间必须串行，否则 `active_count` 互相干扰。
    fn serial() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    fn sleeper() -> Child {
        std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("启动 sleep 失败")
    }

    #[test]
    fn cancel_marks_only_that_session() {
        let _guard = serial();
        shutdown_all();
        let first = Arc::new(Cancel::default());
        let second = Arc::new(Cancel::default());
        let ids = with_sessions(|map| {
            let mut ids = Vec::new();
            for cancel in [&first, &second] {
                let id = next_id();
                map.insert(
                    id,
                    Session {
                        cancel: cancel.clone(),
                        child: Arc::new(Mutex::new(Some(sleeper()))),
                        socket: None,
                    },
                );
                ids.push(id);
            }
            ids
        });
        assert_eq!(active_count(), 2);

        // 取消不存在的 id 不报错，也不影响别的会话
        super::cancel(0);
        assert!(!first.is_cancelled() && !second.is_cancelled());

        super::cancel(ids[0]);
        assert!(first.is_cancelled(), "要取消的是这一条");
        assert!(!second.is_cancelled(), "别的 id 不能受到影响");
        shutdown_all();
    }

    #[test]
    fn shutdown_all_is_idempotent_and_kills_processes() {
        let _guard = serial();
        shutdown_all();
        let cancel = Arc::new(Cancel::default());
        let child = Arc::new(Mutex::new(Some(sleeper())));
        with_sessions(|map| {
            map.insert(
                next_id(),
                Session {
                    cancel: cancel.clone(),
                    child: child.clone(),
                    socket: None,
                },
            );
        });
        shutdown_all();
        assert!(
            cancel.is_cancelled(),
            "退出时置取消，避免监控线程继续等窗口"
        );
        assert_eq!(active_count(), 0);
        assert!(child.lock().unwrap_or_else(|e| e.into_inner()).is_none());
        // 再来一次不能 panic
        shutdown_all();
        assert_eq!(active_count(), 0);
    }

    #[test]
    fn shutdown_all_removes_the_ipc_socket() {
        let _guard = serial();
        shutdown_all();
        let dir = std::env::temp_dir().join(format!("kichi-sock-clean-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let socket = dir.join("s.sock");
        std::fs::write(&socket, b"").expect("造假 socket 文件");

        let cancel = Arc::new(Cancel::default());
        with_sessions(|map| {
            map.insert(
                next_id(),
                Session {
                    cancel: cancel.clone(),
                    child: Arc::new(Mutex::new(Some(sleeper()))),
                    socket: Some(socket.clone()),
                },
            );
        });
        shutdown_all();
        assert!(
            !socket.exists(),
            "退出收尾要删掉 socket，不能让 /tmp/kichi-play 越攒越多"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn spawn_failure_is_reported() {
        let _guard = serial();
        shutdown_all();
        let config = PlayerConfig {
            program: "kichi-no-such-mpv".to_owned(),
            pre_args: Vec::new(),
            access_args: Vec::new(),
        };
        assert!(config.is_mpv(), "测试命令要按 mpv 路径处理");
        let cancel = Arc::new(Cancel::default());
        let result = start(
            &config,
            "https://cdn.invalid/v?sign=S",
            "t",
            cancel,
            PlaybackExtras::default(),
            |_| {},
        );
        assert!(result.is_err(), "spawn 失败必须冒泡给提示条");
        assert_eq!(active_count(), 0, "失败不能留下登记项");
    }

    #[test]
    fn non_mpv_launch_reports_ready_immediately() {
        let _guard = serial();
        shutdown_all();
        // 非 mpv 路径不建会话、不等窗口：spawn 成功就回投 Ready
        let config = PlayerConfig {
            program: "true".to_owned(),
            pre_args: Vec::new(),
            access_args: Vec::new(),
        };
        let cancel = Arc::new(Cancel::default());
        let (sender, receiver) = std::sync::mpsc::channel();
        let id = start(
            &config,
            "https://cdn.invalid/v?sign=S",
            "t",
            cancel,
            PlaybackExtras::default(),
            move |event| {
                let _ = sender.send(event);
            },
        )
        .expect("启动应当成功");
        assert_eq!(id, 0);
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(2)).ok(),
            Some(StartEvent::Ready {
                subtitle_note: None
            })
        );
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(2)).ok(),
            Some(StartEvent::Stopped { progress: None }),
            "播放器进程退出后要清掉「播放中」"
        );
        assert_eq!(active_count(), 0);
    }

    /// 假 mpv：读 `--input-ipc-server`，起 socket，收到 loadfile 后按 `--fake-mode=` 发事件。
    ///
    /// 用脚本而不是 Rust 里的 UnixListener，是为了把「spawn → 连 socket →
    /// 发 loadfile → 观测 → 收尾」整条链路真跑一遍（mpv 的 socket 是它自己建的）。
    const FAKE_MPV: &str = r#"import json, os, socket, sys, time
sock = None
mode = "ready"
log = None
for arg in sys.argv[1:]:
    if arg.startswith("--input-ipc-server="):
        sock = arg.split("=", 1)[1]
    elif arg.startswith("--fake-mode="):
        mode = arg.split("=", 1)[1]
    elif arg.startswith("--fake-log="):
        log = arg.split("=", 1)[1]
if not sock:
    sys.exit(2)
if os.path.exists(sock):
    os.remove(sock)
server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
server.bind(sock)
server.listen(1)
server.settimeout(20)
try:
    conn, _ = server.accept()
except socket.timeout:
    sys.exit(3)
conn.settimeout(0.2)
def send(obj):
    conn.sendall((json.dumps(obj) + "\n").encode())
buffer = b""
loads = 0
deadline = time.time() + 30
while time.time() < deadline:
    try:
        data = conn.recv(65536)
    except socket.timeout:
        continue
    if not data:
        break
    buffer += data
    while b"\n" in buffer:
        line, buffer = buffer.split(b"\n", 1)
        msg = json.loads(line)
        cmd = msg.get("command") or []
        if cmd and cmd[0] == "observe_property":
            send({"request_id": 0, "error": "success"})
        elif cmd and cmd[0] == "loadfile":
            if mode in ("ready", "sub-fail", "progress"):
                time.sleep(0.2)
                if mode == "progress":
                    send({"event": "property-change", "name": "duration", "data": 120.0})
                    send({"event": "property-change", "name": "time-pos", "data": 42.0})
                send({"event": "property-change", "name": "vo-configured", "data": True})
            elif mode == "fail":
                time.sleep(0.1)
                send({"event": "end-file", "reason": "error"})
            elif mode == "fail-once":
                # 第一次拒绝（CDN 还没准备好），第二次同一个链接就成功
                loads += 1
                time.sleep(0.05)
                if loads == 1:
                    send({"event": "log-message", "level": "warn", "text": "http: HTTP error 403 Forbidden"})
                    send({"event": "end-file", "reason": "error", "file_error": "loading failed"})
                else:
                    time.sleep(0.05)
                    send({"event": "property-change", "name": "vo-configured", "data": True})
            elif mode == "never":
                pass  # 一直不出窗口：测「取消关掉还没出窗口的 mpv」
        elif cmd and cmd[0] == "sub-add":
            if log:
                with open(log, "a") as f:
                    f.write("%s\t%s\n" % (cmd[2] if len(cmd) > 2 else "",
                                          cmd[3] if len(cmd) > 3 else ""))
            if mode == "sub-fail":
                send({"request_id": msg.get("request_id"), "error": "loading failed"})
            else:
                send({"request_id": msg.get("request_id"), "error": "success"})
        elif cmd and cmd[0] == "seek":
            if log:
                with open(log, "a") as f:
                    f.write("seek\t%s\n" % (cmd[1] if len(cmd) > 1 else ""))
            send({"request_id": 0, "error": "success"})
        elif cmd and cmd[0] == "quit":
            os.remove(sock)
            sys.exit(0)
        elif cmd and cmd[0] == "set_property":
            send({"request_id": 0, "error": "success"})
"#;

    /// 一条「参数里带 mpv、实际跑假 mpv 脚本」的播放器命令。
    ///
    /// `is_mpv` 看的是程序名和前置参数，脚本名 `fake_mpv.py` 就含 mpv；
    /// 真正执行的是 python3 脚本，不依赖可执行位。
    fn fake_mpv_config(mode: &str) -> PlayerConfig {
        let dir = std::env::temp_dir().join(format!("kichi-fake-mpv-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let script = dir.join("fake_mpv.py");
        std::fs::write(&script, FAKE_MPV).expect("写假 mpv 脚本");
        PlayerConfig {
            program: "python3".to_owned(),
            pre_args: vec![
                script.to_string_lossy().into_owned(),
                format!("--fake-mode={mode}"),
            ],
            // access_args 是真正会传给播放器的参数；这里留空，避免 python 把
            // `--force-media-title=` 当自己的选项
            access_args: Vec::new(),
        }
    }

    /// 同 [`fake_mpv_config`]，但把收到的 `sub-add`（flags + title）追加写到 `log`。
    fn fake_mpv_config_with_log(mode: &str, log: &std::path::Path) -> PlayerConfig {
        let mut config = fake_mpv_config(mode);
        config
            .pre_args
            .push(format!("--fake-log={}", log.display()));
        config
    }

    #[test]
    fn mpv_reports_ready_when_the_window_appears() {
        let _guard = serial();
        shutdown_all();
        let config = fake_mpv_config("ready");
        let cancel = Arc::new(Cancel::default());
        let (sender, receiver) = std::sync::mpsc::channel();
        start(
            &config,
            "https://cdn.invalid/v?sign=S",
            "测试视频",
            cancel,
            PlaybackExtras::default(),
            move |event| {
                let _ = sender.send(event);
            },
        )
        .expect("启动假 mpv 失败");
        let event = receiver
            .recv_timeout(Duration::from_secs(10))
            .expect("应当回投事件");
        assert_eq!(
            event,
            StartEvent::Ready {
                subtitle_note: None
            }
        );
        // Ready 之后监控线程还在等它退出（这里它会一直挂着直到退出收尾）
        shutdown_all();
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(3)).ok(),
            Some(StartEvent::Stopped { progress: None }),
            "主动关闭已经开播的播放器也要回投 Stopped"
        );
        assert_eq!(active_count(), 0, "退出收尾要把会话清干净");
    }

    #[test]
    fn mpv_seeks_to_resume_and_reports_progress_on_stop() {
        let _guard = serial();
        shutdown_all();
        let dir = std::env::temp_dir().join(format!("kichi-progress-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let log = dir.join("mpv.log");
        let _ = std::fs::remove_file(&log);
        let config = fake_mpv_config_with_log("progress", &log);
        let cancel = Arc::new(Cancel::default());
        let (sender, receiver) = std::sync::mpsc::channel();
        start(
            &config,
            "https://cdn.invalid/v?sign=S",
            "续播",
            cancel,
            PlaybackExtras {
                resume: Some(PlaybackProgress {
                    seconds: 40,
                    duration: 120,
                }),
                ..PlaybackExtras::default()
            },
            move |event| {
                let _ = sender.send(event);
            },
        )
        .expect("启动假 mpv 失败");
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(10)).ok(),
            Some(StartEvent::Ready {
                subtitle_note: None
            })
        );
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(3)).ok(),
            Some(StartEvent::Progress {
                progress: PlaybackProgress {
                    seconds: 42,
                    duration: 120
                }
            })
        );
        shutdown_all();
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(3)).ok(),
            Some(StartEvent::Stopped {
                progress: Some(PlaybackProgress {
                    seconds: 42,
                    duration: 120
                })
            })
        );
        let text = std::fs::read_to_string(&log).unwrap_or_default();
        assert!(text.contains("seek\t40"), "应当跳到历史位置：{text:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn test_assets() -> (SubtitleAssets, std::path::PathBuf) {
        let mut assets = SubtitleAssets::create().expect("建字幕临时目录");
        assets
            .push(
                "srt",
                b"1\n00:00:01,000 --> 00:00:02,000\nhi\n",
                "测试.zh.srt",
            )
            .expect("写字幕");
        let dir = assets
            .tracks()
            .first()
            .and_then(|track| track.path.parent())
            .map(std::path::Path::to_path_buf)
            .expect("字幕有临时目录");
        (assets, dir)
    }

    #[test]
    fn subtitles_are_attached_before_ready_and_cleaned_up() {
        let _guard = serial();
        shutdown_all();
        let config = fake_mpv_config("ready");
        let cancel = Arc::new(Cancel::default());
        let (assets, dir) = test_assets();
        let (sender, receiver) = std::sync::mpsc::channel();
        start(
            &config,
            "https://cdn.invalid/v?sign=S",
            "带字幕",
            cancel,
            PlaybackExtras {
                subtitles: Some(assets),
                ..PlaybackExtras::default()
            },
            move |event| {
                let _ = sender.send(event);
            },
        )
        .expect("启动假 mpv 失败");
        let event = receiver
            .recv_timeout(Duration::from_secs(10))
            .expect("应当回投事件");
        assert_eq!(
            event,
            StartEvent::Ready {
                subtitle_note: None
            }
        );
        shutdown_all();
        let deadline = Instant::now() + Duration::from_secs(3);
        while dir.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(!dir.exists(), "mpv 退出后字幕临时目录要删掉");
    }

    #[test]
    fn a_rejected_sub_add_becomes_a_ready_note() {
        let _guard = serial();
        shutdown_all();
        let config = fake_mpv_config("sub-fail");
        let cancel = Arc::new(Cancel::default());
        let (assets, _dir) = test_assets();
        let (sender, receiver) = std::sync::mpsc::channel();
        start(
            &config,
            "https://cdn.invalid/v?sign=S",
            "字幕被拒",
            cancel,
            PlaybackExtras {
                subtitles: Some(assets),
                ..PlaybackExtras::default()
            },
            move |event| {
                let _ = sender.send(event);
            },
        )
        .expect("启动假 mpv 失败");
        let event = receiver
            .recv_timeout(Duration::from_secs(10))
            .expect("应当回投事件");
        match event {
            StartEvent::Ready { subtitle_note } => {
                let note = subtitle_note.expect("回执失败要带说明");
                assert!(note.contains("字幕未能加载"), "{note}");
            }
            other => panic!("字幕失败不能拖垮播放：{other:?}"),
        }
        shutdown_all();
    }

    #[test]
    fn late_subtitles_are_attached_while_playing() {
        // 预等 2 秒没下完的字幕：播放中放进共享槽，监控线程要自己补挂
        let _guard = serial();
        shutdown_all();
        let dir = std::env::temp_dir().join(format!("kichi-late-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let log = dir.join("sub-add.log");
        let _ = std::fs::remove_file(&log);
        let config = fake_mpv_config_with_log("ready", &log);
        let slot: crate::player::subtitles::SubtitleSlot = Arc::new(Mutex::new(None));
        let late_cancel = Arc::new(Cancel::default());
        let cancel = Arc::new(Cancel::default());
        let (sender, receiver) = std::sync::mpsc::channel();
        start(
            &config,
            "https://cdn.invalid/v?sign=S",
            "补挂",
            cancel,
            PlaybackExtras {
                late: Some(LateSubtitles::new(slot.clone(), late_cancel.clone())),
                ..PlaybackExtras::default()
            },
            move |event| {
                let _ = sender.send(event);
            },
        )
        .expect("启动假 mpv 失败");
        let event = receiver
            .recv_timeout(Duration::from_secs(10))
            .expect("应当回投事件");
        assert_eq!(
            event,
            StartEvent::Ready {
                subtitle_note: None
            }
        );
        // 预等阶段没有字幕，播放开始后才“下完”
        let (assets, sub_dir) = test_assets();
        *slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(assets);
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut text = String::new();
        while Instant::now() < deadline {
            text = std::fs::read_to_string(&log).unwrap_or_default();
            if !text.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(text.contains("测试.zh.srt"), "播放中应当补挂：{text:?}");
        assert!(
            text.starts_with("select\t"),
            "一条都没挂过时，补挂的第一条要选中：{text:?}"
        );
        shutdown_all();
        let deadline = Instant::now() + Duration::from_secs(3);
        while sub_dir.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(!sub_dir.exists(), "补挂的字幕临时目录也要清理");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cancel_before_the_window_appears_quits_mpv() {
        let _guard = serial();
        shutdown_all();
        // 一直不出窗口：正是用户按「取消」的那种状态
        let config = fake_mpv_config("never");
        let cancel = Arc::new(Cancel::default());
        let (sender, receiver) = std::sync::mpsc::channel();
        start(
            &config,
            "https://cdn.invalid/v?sign=S",
            "慢启动",
            cancel.clone(),
            PlaybackExtras::default(),
            move |event| {
                let _ = sender.send(event);
            },
        )
        .expect("启动假 mpv 失败");
        // 等监控线程真的连上并发出 loadfile（fake mpv 收到才会一直不吭声）
        std::thread::sleep(Duration::from_millis(400));
        let started = Instant::now();
        cancel.cancel();
        let event = receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("取消后应当回投事件");
        assert_eq!(event, StartEvent::Cancelled);
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "取消收尾不该拖太久：{:?}",
            started.elapsed()
        );
        // 监控线程收尾后会移除会话；mpv 已被 quit（800 ms 宽限内）
        let deadline = Instant::now() + Duration::from_secs(3);
        while active_count() > 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(active_count(), 0, "取消后会话要从登记表移除");
    }

    #[test]
    fn mpv_reporting_no_ipc_but_exiting_is_a_failure() {
        let _guard = serial();
        shutdown_all();
        // 名字含 mpv、但根本不建 socket 就直接退出（相当于播放器启动失败）
        let config = PlayerConfig {
            program: "false".to_owned(),
            pre_args: vec!["fake_mpv".to_owned()],
            access_args: Vec::new(),
        };
        let cancel = Arc::new(Cancel::default());
        let (sender, receiver) = std::sync::mpsc::channel();
        start(
            &config,
            "https://cdn.invalid/v?sign=S",
            "起不来",
            cancel,
            PlaybackExtras::default(),
            move |event| {
                let _ = sender.send(event);
            },
        )
        .expect("spawn 本身能成功");
        let event = receiver
            .recv_timeout(Duration::from_secs(10))
            .expect("应当回投事件");
        match event {
            StartEvent::Failed(text) => assert!(text.contains("起不来"), "{text}"),
            other => panic!("播放器没起来应当报失败：{other:?}"),
        }
    }

    #[test]
    fn a_rejected_link_is_retried_and_then_plays() {
        let _guard = serial();
        shutdown_all();
        // 第一次 403、第二次成功：用户看到的就是「等一下自己播起来」
        let config = fake_mpv_config("fail-once");
        let cancel = Arc::new(Cancel::default());
        let (sender, receiver) = std::sync::mpsc::channel();
        start(
            &config,
            "https://cdn.invalid/v?sign=S",
            "第一次被拒",
            cancel,
            PlaybackExtras::default(),
            move |event| {
                let _ = sender.send(event);
            },
        )
        .expect("启动假 mpv 失败");
        let event = receiver
            .recv_timeout(Duration::from_secs(10))
            .expect("应当回投事件");
        assert_eq!(
            event,
            StartEvent::Ready {
                subtitle_note: None
            },
            "重试后应当成功开播"
        );
        shutdown_all();
    }

    #[test]
    fn mpv_reports_failure_for_a_bad_link() {
        let _guard = serial();
        shutdown_all();
        let config = fake_mpv_config("fail");
        let cancel = Arc::new(Cancel::default());
        let (sender, receiver) = std::sync::mpsc::channel();
        start(
            &config,
            "https://cdn.invalid/v?sign=S",
            "坏链接",
            cancel,
            PlaybackExtras::default(),
            move |event| {
                let _ = sender.send(event);
            },
        )
        .expect("启动假 mpv 失败");
        let event = receiver
            .recv_timeout(Duration::from_secs(10))
            .expect("应当回投事件");
        match event {
            StartEvent::Failed(text) => assert!(text.contains("坏链接"), "{text}"),
            other => panic!("坏链接应当失败：{other:?}"),
        }
        shutdown_all();
        assert_eq!(active_count(), 0);
    }
}
