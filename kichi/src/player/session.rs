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

use std::collections::HashMap;
use std::process::Child;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier, Mutex, OnceLock};
use std::time::{Duration, Instant};

use kichi_core::PlaybackProgress;

use super::ipc::{self, StartOutcome};
use super::progress::ProgressTracker;
use super::subtitles::{self, LateSubtitles, SubtitleAssets};
use super::{launch, mask_url, vlc, PlayerConfig, PlayerProtocol};
use crate::session::download::Cancel;

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

/// 启动 VLC：私有播放清单 + oldrc socket + 播放监控线程。
fn start_vlc(
    config: &PlayerConfig,
    url: &str,
    title: &str,
    cancel: Arc<Cancel>,
    extras: PlaybackExtras,
    notify: impl Fn(StartEvent) + Send + Sync + 'static,
) -> Result<u64, String> {
    let notify = Arc::new(notify);
    let playlist = vlc::write_playlist(url, title)?;
    let socket = ipc::socket_path();
    let _ = std::fs::remove_file(&socket);

    let mut command = vlc::build_command(config, &socket, &playlist, title);
    let child = match command.spawn() {
        Ok(child) => child,
        Err(err) => {
            let _ = std::fs::remove_file(&playlist);
            let _ = std::fs::remove_file(&socket);
            return Err(format!("启动播放器 {} 失败：{err}", config.program()));
        }
    };
    eprintln!(
        "[kichi] 启动 VLC 播放「{title}」→ {}（socket {}）",
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
                cleanup: vec![socket.clone(), playlist.clone()],
            },
        );
    });

    let PlaybackExtras { resume, .. } = extras;
    let program = config.program().to_owned();
    let title = title.to_owned();
    let monitor_socket = socket.clone();
    let monitor_playlist = playlist.clone();
    let monitor_child = child.clone();
    let monitor_notify = notify.clone();
    let event_notify = monitor_notify.clone();
    let spawned = std::thread::Builder::new()
        .name("kichi-vlc".into())
        .spawn(move || {
            let outcome = run_vlc_monitor(
                &monitor_socket,
                &monitor_playlist,
                &monitor_child,
                &cancel,
                &title,
                &program,
                resume,
                move |event| event_notify(event),
            );
            if outcome.was_ready {
                monitor_notify(StartEvent::Stopped {
                    progress: outcome.final_progress,
                    end: outcome.end,
                });
            }
            with_sessions(|map| map.remove(&id));
            let _ = std::fs::remove_file(&monitor_socket);
            let _ = std::fs::remove_file(&monitor_playlist);
        });
    if let Err(err) = spawned {
        with_sessions(|map| map.remove(&id));
        let _ = std::fs::remove_file(&socket);
        let _ = std::fs::remove_file(&playlist);
        kill_child(&child);
        return Err(format!("启动 VLC 播放监控线程失败：{err}"));
    }
    Ok(id)
}

// 参数都是监控线程的固定上下文；拆结构体只会增加一层解包。
#[allow(clippy::too_many_arguments)]
fn run_vlc_monitor(
    socket: &std::path::Path,
    playlist: &std::path::Path,
    child: &Arc<Mutex<Option<Child>>>,
    cancel: &Cancel,
    title: &str,
    program: &str,
    resume: Option<PlaybackProgress>,
    emit: impl Fn(StartEvent),
) -> MonitorOutcome {
    let Some(mut conn) = vlc::connect(socket, cancel) else {
        if cancel.is_cancelled() {
            kill_child(child);
            emit(StartEvent::Cancelled);
            return MonitorOutcome {
                was_ready: false,
                final_progress: None,
                end: EndReason::Unknown,
            };
        }
        if child_exited(child) {
            emit(StartEvent::Failed(format!(
                "VLC 未能播放「{title}」：播放器已退出（无法连接控制接口）"
            )));
            return MonitorOutcome {
                was_ready: false,
                final_progress: None,
                end: EndReason::Unknown,
            };
        }
        // 老版本 / 精简版 VLC 可能没有 oldrc。播放本身仍然成功时，
        // 退回只能等进程退出的降级模式，不把已经打开的视频报成失败。
        eprintln!("[kichi] 连不上 VLC 的 oldrc，无法观测播放状态（{program}）");
        emit(StartEvent::Ready {
            subtitle_note: None,
        });
        wait_for_exit(child);
        return MonitorOutcome {
            was_ready: true,
            final_progress: None,
            end: EndReason::Unknown,
        };
    };

    loop {
        if cancel.is_cancelled() {
            shut_down_vlc(&mut conn, child);
            emit(StartEvent::Cancelled);
            return MonitorOutcome {
                was_ready: false,
                final_progress: None,
                end: EndReason::Unknown,
            };
        }
        if child_exited(child) {
            emit(StartEvent::Failed(format!(
                "VLC 未能播放「{title}」：播放器在加载前退出"
            )));
            return MonitorOutcome {
                was_ready: false,
                final_progress: None,
                end: EndReason::Unknown,
            };
        }
        match conn.is_playing() {
            Ok(Some(true)) => break,
            Ok(Some(false) | None) => {}
            Err(err) => {
                if child_exits_within(child, Duration::from_millis(300)) {
                    emit(StartEvent::Failed(format!(
                        "VLC 未能播放「{title}」：播放器在加载前退出"
                    )));
                    return MonitorOutcome {
                        was_ready: false,
                        final_progress: None,
                        end: EndReason::Unknown,
                    };
                }
                eprintln!("[kichi] 读取 VLC 播放状态失败：{err}");
                emit(StartEvent::Ready {
                    subtitle_note: None,
                });
                wait_for_exit(child);
                return MonitorOutcome {
                    was_ready: true,
                    final_progress: None,
                    end: EndReason::Unknown,
                };
            }
        }
        std::thread::sleep(POLL);
    }

    // VLC 已经读入清单，之后窗口生命周期不再依赖这个文件。
    let _ = std::fs::remove_file(playlist);
    if let Some(progress) = resume {
        if let Err(err) = conn.seek_absolute(progress.seconds) {
            eprintln!("[kichi] 恢复 VLC 播放进度失败（只记日志）：{err}");
        }
    }
    emit(StartEvent::Ready {
        subtitle_note: None,
    });
    let (final_progress, end) = wait_vlc_playing(&mut conn, child, cancel, resume, &emit);
    MonitorOutcome {
        was_ready: true,
        final_progress,
        end,
    }
}

/// 根据跟踪器与取消标志判断 mpv 的结束原因。
fn end_reason(tracker: &ProgressTracker, cancel: &Cancel) -> EndReason {
    if tracker.reached_end() {
        EndReason::Eof
    } else if cancel.is_cancelled() {
        EndReason::Stopped
    } else {
        EndReason::Unknown
    }
}

/// VLC 没有 EOF 事件：只能按「位置接近片尾 + 进程退出」近似。
fn vlc_end_reason(progress: Option<PlaybackProgress>, cancel: &Cancel) -> EndReason {
    if cancel.is_cancelled() {
        return EndReason::Stopped;
    }
    match progress {
        Some(progress) if progress.duration > 0 && progress.seconds + 2 >= progress.duration => {
            EndReason::Eof
        }
        _ => EndReason::Unknown,
    }
}

fn wait_vlc_playing(
    conn: &mut vlc::Connection,
    child: &Arc<Mutex<Option<Child>>>,
    cancel: &Cancel,
    resume: Option<PlaybackProgress>,
    emit: &impl Fn(StartEvent),
) -> (Option<PlaybackProgress>, EndReason) {
    let mut tracker = ProgressTracker::new(
        None,
        None,
        resume.map(|progress| progress.duration),
        Instant::now(),
    );
    let mut next_poll = Instant::now();
    loop {
        if cancel.is_cancelled() {
            shut_down_vlc(conn, child);
            return (tracker.final_progress(), EndReason::Stopped);
        }
        if child_exited(child) {
            let progress = tracker.final_progress();
            return (progress, vlc_end_reason(progress, cancel));
        }
        if Instant::now() >= next_poll {
            let position = conn.position();
            let duration = conn.duration();
            match (position, duration) {
                (Ok(position), Ok(duration)) => tracker.update_values(position, duration),
                (Err(err), _) | (_, Err(err)) => {
                    eprintln!("[kichi] 读取 VLC 播放进度失败：{err}");
                }
            }
            if let Some(progress) = tracker.maybe_emit(Instant::now()) {
                emit(StartEvent::Progress { progress });
            }
            next_poll = Instant::now() + VLC_POLL;
        }
        std::thread::sleep(POLL);
    }
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
            end: outcome.end,
        });
    }
}

struct MonitorOutcome {
    was_ready: bool,
    final_progress: Option<PlaybackProgress>,
    end: EndReason,
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
                end: EndReason::Unknown,
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
                end: EndReason::Unknown,
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
            end: EndReason::Unknown,
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
            end: EndReason::Unknown,
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
                    end: EndReason::Unknown,
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
                        end: EndReason::Unknown,
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
                    end: EndReason::Unknown,
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
            let (final_progress, end) = wait_playing(&mut conn, child, cancel, hooks);
            MonitorOutcome {
                was_ready: true,
                final_progress,
                end,
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
                end: EndReason::Unknown,
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
) -> (Option<PlaybackProgress>, EndReason) {
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
    let mut exited_at = None;
    loop {
        if cancel.is_cancelled() {
            shut_down(conn, child);
            return (tracker.final_progress(), EndReason::Stopped);
        }
        if child_exited(child) && exited_at.is_none() {
            exited_at = Some(Instant::now());
        }
        if exited_at
            .is_some_and(|at| Instant::now().saturating_duration_since(at) >= EXIT_EVENT_GRACE)
        {
            return (tracker.final_progress(), end_reason(&tracker, cancel));
        }
        if exited_at.is_none() {
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
        }
        match conn.read_playback_event() {
            Ok(ipc::PlaybackRead::Message(value)) => tracker.observe(&value),
            Ok(ipc::PlaybackRead::Timeout) => {}
            Ok(ipc::PlaybackRead::Closed) => {
                // 进程已退出时，把 socket 里最后几条事件（尤其 end-file eof）读完。
                if exited_at.is_some() {
                    return (tracker.final_progress(), end_reason(&tracker, cancel));
                }
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

fn shut_down_vlc(conn: &mut vlc::Connection, child: &Arc<Mutex<Option<Child>>>) {
    if let Err(err) = conn.quit() {
        eprintln!("[kichi] 请求 VLC 退出失败：{err}");
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

    fn wait_for_no_sessions() {
        let deadline = Instant::now() + Duration::from_secs(2);
        while active_count() > 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
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
                        cleanup: Vec::new(),
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
                    cleanup: Vec::new(),
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
                    cleanup: vec![socket.clone()],
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
    fn mpv_input_placeholders_are_rejected() {
        let _guard = serial();
        shutdown_all();
        let config = PlayerConfig {
            program: "mpv".to_owned(),
            pre_args: Vec::new(),
            access_args: vec!["{url}".to_owned()],
        };
        let result = start(
            &config,
            "https://cdn.invalid/v?sign=S",
            "t",
            Arc::new(Cancel::default()),
            PlaybackExtras::default(),
            |_| {},
        );
        assert!(
            result
                .err()
                .is_some_and(|reason| reason.contains("不能使用 {url}")),
            "mpv 的输入必须由 IPC 下发"
        );
        assert_eq!(active_count(), 0);
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
            Some(StartEvent::Stopped {
                progress: None,
                end: EndReason::Unknown,
            }),
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
            if mode in ("ready", "sub-fail", "progress", "complete"):
                time.sleep(0.2)
                if mode == "progress":
                    send({"event": "property-change", "name": "duration", "data": 120.0})
                    send({"event": "property-change", "name": "time-pos", "data": 42.0})
                elif mode == "complete":
                    send({"event": "property-change", "name": "duration", "data": 120.0})
                    send({"event": "property-change", "name": "time-pos", "data": 119.2})
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
            if mode == "complete":
                send({"event": "end-file", "reason": "eof"})
                time.sleep(0.01)
                sys.exit(0)
            else:
                send({"request_id": 0, "error": "success"})
"#;

    /// 假 VLC：只实现 Kichi 用到的 oldrc 数值命令。
    const FAKE_VLC: &str = r#"import os, socket, sys
sock = None
log = None
mode = "ready"
for arg in sys.argv[1:]:
    if arg.startswith("--rc-unix="):
        sock = arg.split("=", 1)[1]
    elif arg.startswith("--fake-log="):
        log = arg.split("=", 1)[1]
    elif arg.startswith("--fake-mode="):
        mode = arg.split("=", 1)[1]
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
buffer = b""
position = 0
while True:
    try:
        data = conn.recv(65536)
    except socket.timeout:
        continue
    if not data:
        break
    buffer += data
    while b"\n" in buffer:
        line, buffer = buffer.split(b"\n", 1)
        cmd = line.decode(errors="replace").strip()
        if cmd == "is_playing":
            if mode == "fail":
                sys.exit(4)
            conn.sendall(b"1\n")
        elif cmd == "get_time":
            position += 2
            conn.sendall((str(position) + "\n").encode())
        elif cmd == "get_length":
            conn.sendall(b"120\n")
        elif cmd.startswith("seek "):
            if log:
                with open(log, "a") as f:
                    f.write("seek\t%s\n" % cmd.split(" ", 1)[1])
            conn.sendall(b"seek: returned 0 (no error)\n")
        elif cmd == "quit":
            os.remove(sock)
            sys.exit(0)
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

    fn fake_vlc_config(mode: &str, log: &std::path::Path) -> PlayerConfig {
        let dir = std::env::temp_dir().join(format!("kichi-fake-vlc-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let script = dir.join("fake_vlc.py");
        std::fs::write(&script, FAKE_VLC).expect("写假 VLC 脚本");
        PlayerConfig {
            program: "python3".to_owned(),
            pre_args: vec![
                script.to_string_lossy().into_owned(),
                format!("--fake-mode={mode}"),
                format!("--fake-log={}", log.display()),
            ],
            access_args: Vec::new(),
        }
    }

    #[test]
    fn vlc_reports_ready_progress_resume_and_stopped() {
        let _guard = serial();
        shutdown_all();
        let log = std::env::temp_dir().join(format!("kichi-fake-vlc-{}.log", std::process::id()));
        let _ = std::fs::remove_file(&log);
        let config = fake_vlc_config("ready", &log);
        assert_eq!(config.protocol(), PlayerProtocol::Vlc);
        let (sender, receiver) = std::sync::mpsc::channel();
        let id = start(
            &config,
            "https://cdn.invalid/v?sign=S",
            "测试视频",
            Arc::new(Cancel::default()),
            PlaybackExtras {
                resume: Some(PlaybackProgress {
                    seconds: 42,
                    duration: 120,
                }),
                ..PlaybackExtras::default()
            },
            move |event| {
                let _ = sender.send(event);
            },
        )
        .expect("启动假 VLC 失败");
        assert_ne!(id, 0);
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(2)).ok(),
            Some(StartEvent::Ready {
                subtitle_note: None
            })
        );
        std::thread::sleep(Duration::from_millis(260));
        shutdown_all();

        let mut saw_progress = false;
        let final_progress = loop {
            match receiver.recv_timeout(Duration::from_secs(2)).ok() {
                Some(StartEvent::Progress { .. }) => saw_progress = true,
                Some(StartEvent::Stopped { progress, .. }) => break progress,
                Some(other) => panic!("意外事件：{other:?}"),
                None => break None,
            }
        };
        assert!(saw_progress, "VLC 轮询应该产生播放进度");
        assert!(final_progress.is_some(), "退出时要补最后一个 VLC 播放位置");
        let log = std::fs::read_to_string(&log).expect("读取假 VLC 日志");
        assert!(log.contains("seek\t42"), "续播位置要走 RC seek：{log}");
        let _ = std::fs::remove_file(log);
        assert_eq!(active_count(), 0);
    }

    #[test]
    fn vlc_exit_before_ready_is_reported() {
        let _guard = serial();
        shutdown_all();
        let log =
            std::env::temp_dir().join(format!("kichi-fake-vlc-fail-{}.log", std::process::id()));
        let config = fake_vlc_config("fail", &log);
        let (sender, receiver) = std::sync::mpsc::channel();
        start(
            &config,
            "https://cdn.invalid/v?sign=S",
            "坏视频",
            Arc::new(Cancel::default()),
            PlaybackExtras::default(),
            move |event| {
                let _ = sender.send(event);
            },
        )
        .expect("启动假 VLC 失败");
        match receiver.recv_timeout(Duration::from_secs(2)).ok() {
            Some(StartEvent::Failed(text)) => assert!(text.contains("加载前退出"), "{text}"),
            other => panic!("应当报 VLC 加载失败，实际 {other:?}"),
        }
        // 失败事件是在监控线程里发出的；会话表要等回调返回后才移除。
        wait_for_no_sessions();
        assert_eq!(active_count(), 0, "加载失败的会话要从登记表移除");
        let _ = std::fs::remove_file(log);
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
            Some(StartEvent::Stopped {
                progress: None,
                end: EndReason::Stopped,
            }),
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
                }),
                end: EndReason::Stopped,
            })
        );
        let text = std::fs::read_to_string(&log).unwrap_or_default();
        assert!(text.contains("seek\t40"), "应当跳到历史位置：{text:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mpv_natural_eof_reports_last_non_full_second() {
        let _guard = serial();
        shutdown_all();
        let config = fake_mpv_config("complete");
        let cancel = Arc::new(Cancel::default());
        let (sender, receiver) = std::sync::mpsc::channel();
        start(
            &config,
            "https://cdn.invalid/v?sign=S",
            "播完",
            cancel,
            PlaybackExtras::default(),
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
        let final_progress = loop {
            match receiver.recv_timeout(Duration::from_secs(3)) {
                Ok(StartEvent::Progress { .. }) => continue,
                Ok(StartEvent::Stopped { progress, .. }) => break progress,
                other => panic!("意外的播放事件：{other:?}"),
            }
        };
        assert_eq!(
            final_progress,
            Some(PlaybackProgress {
                seconds: 119,
                duration: 120
            }),
            "自然播完必须上报片尾前一秒，避免服务端把满进度归零"
        );
        wait_for_no_sessions();
        assert_eq!(active_count(), 0);
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
