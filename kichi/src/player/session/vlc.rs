//! VLC 监控循环：oldrc socket → 等 `is_playing` → 轮询进度 → 等退出。

use std::process::Child;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kichi_core::PlaybackProgress;

use crate::player::progress::ProgressTracker;
use crate::player::{ipc, mask_url, vlc, PlayerConfig};
use crate::session::download::Cancel;

use super::{
    child_exited, child_exits_within, kill_child, next_id, wait_for_exit, with_sessions, EndReason,
    MonitorOutcome, PlaybackExtras, Session, StartEvent, POLL, QUIT_GRACE, VLC_POLL,
};

/// 启动 VLC：私有播放清单 + oldrc socket + 播放监控线程。
pub(super) fn start_vlc(
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
