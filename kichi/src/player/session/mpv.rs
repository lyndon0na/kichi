//! mpv 监控循环：IPC 就绪 → 挂字幕 / 续播 → 等退出。

use std::process::Child;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kichi_core::PlaybackProgress;

use crate::player::ipc::{self, StartOutcome};
use crate::player::progress::ProgressTracker;
use crate::player::subtitles::{self, LateSubtitles, SubtitleAssets};
use crate::session::download::Cancel;

use super::{
    child_exited, kill_child, wait_for_exit, EndReason, MonitorOutcome, StartEvent,
    EXIT_EVENT_GRACE, POLL, QUIT_GRACE,
};

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

/// 监控线程主体：连接 → observe + loadfile → 等就绪 → 收尾。
// 参数都是监控线程的固定上下文（socket / 进程 / 取消 / 直链 / 标题 / 程序名 /
// 字幕资产 / 回投），拆成结构体只会多一层解包，没有可读性收益。
#[allow(clippy::too_many_arguments)]
pub(super) fn run_monitor(
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
