//! libmpv 生命周期与事件循环（运行在独立的 `kichi-embed-mpv` 线程）。
//!
//! 只把 URL 通过 `loadfile` 命令交给 mpv：不进 argv、不落盘，日志只打脱敏后的主机名。

use std::sync::atomic::Ordering;
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::Instant;

use libmpv2::events::{Event, PropertyData};
use libmpv2::{Format, Mpv};

use super::{AudioSettings, Control, Session, WaylandSocketGuard};
use crate::player::progress::ProgressTracker;
use crate::player::subtitles::{take_slot, SubtitleAssets};
use crate::player::{PlaybackExtras, StartEvent};

/// 事件循环最长阻塞时间；界面控制命令靠它及时生效。
const EVENT_TIMEOUT: f64 = 0.05;

// 启动参数共同描述一次会话，拆结构只会让调用点来回打包。
#[allow(clippy::too_many_arguments)]
pub(super) fn run(
    session: Arc<Session>,
    url: &str,
    title: &str,
    extras: PlaybackExtras,
    audio: AudioSettings,
    notify: impl Fn(StartEvent),
    control_rx: Receiver<Control>,
    guard: WaylandSocketGuard,
) {
    ensure_c_locale();
    let PlaybackExtras {
        subtitles,
        late,
        resume,
    } = extras;

    let mpv = match create_mpv(title, audio) {
        Ok(mpv) => mpv,
        Err(err) => {
            eprintln!("[kichi] 内嵌播放器初始化失败：{err}");
            notify(StartEvent::Failed(format!("内嵌播放器初始化失败：{err}")));
            return;
        }
    };
    if let Err(err) = mpv.command("loadfile", &[url, "replace"]) {
        eprintln!("[kichi] 内嵌播放器加载失败：{err}");
        notify(StartEvent::Failed(format!(
            "内嵌播放器未能播放「{title}」：{err}"
        )));
        terminate(mpv);
        return;
    }

    let fallback_duration = resume.map(|progress| progress.duration);
    let mut tracker = ProgressTracker::new(
        resume.map(|progress| progress.seconds as f64),
        None,
        fallback_duration,
        Instant::now(),
    );
    let mut guard = Some(guard);
    let mut ready = false;
    let mut failure: Option<String> = None;
    let mut next_sub_id = 0u64;
    let mut selected = false;
    let mut late_held: Vec<SubtitleAssets> = Vec::new();
    let mut load_attempts = 0usize;
    let mut http_error: Option<String> = None;

    loop {
        if session.cancel().is_cancelled() || session.proxy_stop.load(Ordering::Acquire) {
            break;
        }
        while let Ok(control) = control_rx.try_recv() {
            match control {
                Control::TogglePause => {
                    let paused = session.is_paused();
                    if let Err(err) = mpv.set_property("pause", !paused) {
                        eprintln!("[kichi] 内嵌播放器切换暂停失败：{err}");
                    }
                }
                Control::SeekAbsolute(seconds) => {
                    let target = format!("{seconds:.3}");
                    if let Err(err) = mpv.command("seek", &[&target, "absolute"]) {
                        eprintln!("[kichi] 内嵌播放器跳转失败：{err}");
                    }
                }
                Control::SetVolume(volume) => {
                    if let Err(err) = mpv.set_property("volume", volume) {
                        eprintln!("[kichi] 内嵌播放器调音量失败：{err}");
                    }
                }
                Control::SetMuted(muted) => {
                    if let Err(err) = mpv.set_property("mute", muted) {
                        eprintln!("[kichi] 内嵌播放器切换静音失败：{err}");
                    }
                }
                Control::ToggleStats => {
                    if let Err(err) = mpv.command("script-binding", &["stats/display-stats-toggle"])
                    {
                        eprintln!("[kichi] 内嵌播放器切换统计信息失败：{err}");
                    }
                }
                Control::Quit => {
                    let _ = mpv.command("quit", &[]);
                }
            }
        }
        // 只有 mpv 的客户端真的连上代理（发来第一个请求），WAYLAND_SOCKET
        // 才能安全恢复。不能按时间到期：慢网络下 mpv 可能几十秒才建连，
        // 提前恢复会让它连上真实合成器，弹出第二个播放器窗口。
        if guard.is_some() && session.wayland_connected() {
            eprintln!("[kichi] 内嵌播放器已连上 Wayland 代理，恢复 WAYLAND_SOCKET");
            guard.take();
        }
        if ready {
            if let Some(late) = &late {
                if let Some(assets) = take_slot(&late.slot) {
                    if !assets.is_empty() {
                        match attach_subtitles(&mpv, &assets, &mut next_sub_id, &mut selected) {
                            None => {
                                eprintln!("[kichi] 字幕：播放中补挂 {} 条", assets.tracks().len())
                            }
                            Some(note) => eprintln!("[kichi] 字幕：播放中补挂失败：{note}"),
                        }
                    }
                    late_held.push(assets);
                }
            }
        }

        match mpv.wait_event(EVENT_TIMEOUT) {
            None => {}
            Some(Ok(Event::FileLoaded)) => {
                if ready {
                    continue;
                }
                let note = subtitles.as_ref().and_then(|assets| {
                    attach_subtitles(&mpv, assets, &mut next_sub_id, &mut selected)
                });
                if let Some(resume) = resume {
                    let target = format!("{:.3}", resume.seconds as f64);
                    if let Err(err) = mpv.command("seek", &[&target, "absolute"]) {
                        eprintln!("[kichi] 恢复播放进度失败（只记日志）：{err}");
                    }
                }
                ready = true;
                notify(StartEvent::Ready {
                    subtitle_note: note,
                });
            }
            Some(Ok(Event::PropertyChange { name, change, .. })) => match (name, change) {
                ("time-pos", PropertyData::Double(position)) => {
                    session.set_position(valid_seconds(position));
                    tracker.update_values(valid_seconds(position), None);
                }
                ("duration", PropertyData::Double(duration)) => {
                    session.set_duration(valid_seconds(duration));
                    tracker.update_values(None, valid_seconds(duration));
                }
                ("pause", PropertyData::Flag(paused)) => session.set_paused(paused),
                ("vo-configured", PropertyData::Flag(true)) => {
                    if !session.wayland_connected() {
                        // VO 配置成功却没走过内嵌代理：mpv 连到了别的地方，
                        // 画面不可能进 kichi 窗口。中止，绝不留黑浮层。
                        failure = Some(format!(
                            "内嵌播放器未能播放「{title}」：播放器没有接管内嵌画面，已中止"
                        ));
                        break;
                    }
                    guard.take();
                }
                _ => {}
            },
            Some(Ok(Event::LogMessage { text, .. })) => {
                // HTTP 状态码只出现在日志里；只留状态码，不碰后面的 URL。
                if let Some(reason) = crate::player::ipc::http_failure_from_text(text) {
                    http_error = Some(reason);
                }
            }
            Some(Ok(Event::EndFile(reason))) => {
                if reason == libmpv2_sys::mpv_end_file_reason_MPV_END_FILE_REASON_EOF {
                    tracker.mark_reached_end();
                    break;
                }
                // 首次打不开多半是 CDN 还没准备好签名直链（403），
                // 同一个链接立刻重来通常就能播（见 ADR 0007）。
                if reason == libmpv2_sys::mpv_end_file_reason_MPV_END_FILE_REASON_ERROR
                    && !ready
                    && load_attempts < crate::player::ipc::LOAD_RETRIES
                {
                    let delay = crate::player::ipc::retry_delay(load_attempts);
                    load_attempts += 1;
                    eprintln!(
                        "[kichi] 内嵌播放加载失败（第 {load_attempts} 次），{delay:?} 后用同一链接重试"
                    );
                    http_error = None;
                    if !crate::session::download::sleep_cancellable(session.cancel(), delay) {
                        break;
                    }
                    if let Err(err) = mpv.command("loadfile", &[url, "replace"]) {
                        failure = Some(format!(
                            "内嵌播放器未能播放「{title}」：无法重新加载（{err}）"
                        ));
                        break;
                    }
                    continue;
                }
                if !ready && reason == libmpv2_sys::mpv_end_file_reason_MPV_END_FILE_REASON_ERROR {
                    failure = Some(match http_error.take() {
                        Some(detail) => {
                            format!("内嵌播放器未能播放「{title}」：链接加载失败（HTTP {detail}）")
                        }
                        None => format!("内嵌播放器未能播放「{title}」：链接加载失败"),
                    });
                }
                break;
            }
            Some(Ok(Event::Shutdown)) => break,
            Some(Ok(_)) => {}
            Some(Err(err)) => {
                if !ready {
                    failure = Some(format!("内嵌播放器未能播放「{title}」：{err}"));
                }
                break;
            }
        }

        if let Some(progress) = tracker.maybe_emit(Instant::now()) {
            notify(StartEvent::Progress { progress });
        }
    }

    if let Some(progress) = tracker.maybe_emit(Instant::now()) {
        notify(StartEvent::Progress { progress });
    }
    if ready {
        notify(StartEvent::Stopped {
            progress: tracker.final_progress(),
        });
    } else if let Some(failure) = failure {
        notify(StartEvent::Failed(failure));
    } else if session.cancel().is_cancelled() {
        notify(StartEvent::Cancelled);
    } else if let Some(reason) = session.proxy_error() {
        notify(StartEvent::Failed(format!(
            "内嵌播放器未能播放「{title}」：{reason}"
        )));
    } else {
        notify(StartEvent::Failed(format!(
            "内嵌播放器未能播放「{title}」：播放提前结束"
        )));
    }

    drop(late_held);
    drop(late);
    drop(subtitles);
    terminate(mpv);
}

fn create_mpv(title: &str, audio: AudioSettings) -> Result<Mpv, libmpv2::Error> {
    let mpv = Mpv::with_initializer(|init| {
        for (name, value) in [
            ("vo", "gpu-next"),
            ("gpu-context", "wayland"),
            ("gpu-api", "opengl"),
            ("hwdec", "auto-safe"),
            // 测试自检时把 mpv 日志打到 stderr，方便排查链路；正式构建保持静默。
            ("terminal", if cfg!(test) { "yes" } else { "no" }),
            // 错误级日志仍要生成：HTTP 403 这类状态码只出现在日志里。
            ("msg-level", if cfg!(test) { "all=v" } else { "all=error" }),
            // 剪贴板只走 VO：默认的 wayland 后端会抢在 VO 之前用掉 WAYLAND_SOCKET。
            ("clipboard-backends", "vo"),
            ("input-default-bindings", "no"),
            ("input-vo-keyboard", "no"),
            ("osc", "no"),
            ("osd-level", "0"),
            ("config", "no"),
            ("idle", "yes"),
            ("force-window", "yes"),
            ("ytdl", "no"),
            ("audio-display", "no"),
            ("sub-auto", "no"),
        ] {
            init.set_option(name, value)?;
        }
        Ok(())
    })?;
    mpv.set_property("force-media-title", title)?;
    // 音量 / 静音在建 mpv 时一次设好，避免 loadfile 后先按 100% 出一声再被命令改掉。
    mpv.set_property("volume", i64::from(audio.volume))?;
    mpv.set_property("mute", audio.muted)?;
    // 自动重试与错误提示都要读 HTTP 状态码；请求错误级日志。
    // SAFETY: ctx 来自刚创建好的 libmpv2 句柄，字符串静态且以 NUL 结尾。
    unsafe {
        libmpv2_sys::mpv_request_log_messages(mpv.ctx.as_ptr(), c"error".as_ptr());
    }
    for (index, (name, format)) in [
        ("time-pos", Format::Double),
        ("duration", Format::Double),
        ("pause", Format::Flag),
        ("vo-configured", Format::Flag),
    ]
    .into_iter()
    .enumerate()
    {
        mpv.observe_property(name, format, index as u64 + 1)?;
    }
    Ok(mpv)
}

/// 挂字幕；返回一句失败原因（成功 / 没有字幕时 `None`）。
fn attach_subtitles(
    mpv: &Mpv,
    assets: &SubtitleAssets,
    _next_sub_id: &mut u64,
    selected: &mut bool,
) -> Option<String> {
    for track in assets.tracks() {
        let path = track.path.to_string_lossy();
        let flag = if *selected { "auto" } else { "select" };
        if let Err(err) = mpv.command("sub-add", &[&path, flag, &track.title]) {
            eprintln!("[kichi] 字幕：sub-add 失败：{err}");
            return Some(format!("字幕未能加载：{err}"));
        }
        *selected = true;
    }
    if assets.is_empty() {
        None
    } else {
        eprintln!("[kichi] 字幕：已挂载 {} 条", assets.tracks().len());
        None
    }
}

fn valid_seconds(value: f64) -> Option<f64> {
    value
        .is_finite()
        .then_some(value)
        .filter(|value| *value >= 0.0)
}

fn ensure_c_locale() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // HACK(LC_NUMERIC)：libmpv 要求 LC_NUMERIC=C；进程范围只设一次。
        unsafe {
            libc::setlocale(libc::LC_NUMERIC, c"C".as_ptr());
        }
    });
}

fn terminate(mpv: Mpv) {
    // libmpv2 的 Drop 只调 mpv_destroy（不等待核心退出）；这里显式等它收尾。
    let ctx = mpv.ctx.as_ptr();
    unsafe { libmpv2_sys::mpv_terminate_destroy(ctx) };
    std::mem::forget(mpv);
}
