//! libmpv 生命周期与事件循环（运行在独立的 `kichi-embed-mpv` 线程）。
//!
//! 只把 URL 通过 `loadfile` 命令交给 mpv：不进 argv、不落盘，日志只打脱敏后的主机名。

use std::sync::atomic::Ordering;
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::Instant;

use libmpv2::events::{Event, PropertyData};
use libmpv2::{mpv_error, Error, Format, Mpv};

use super::{
    color, conf, ChapterInfo, Control, Session, SessionSettings, SubtitleStyle, TrackInfo,
    WaylandSocketGuard,
};
use crate::player::progress::ProgressTracker;
use crate::player::screensaver::ScreenInhibitor;
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
    settings: SessionSettings,
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

    let (mpv, mut current_target) = match create_mpv(title, &settings) {
        Ok(mpv) => mpv,
        Err(err) => {
            let reason = describe_error(&err);
            eprintln!("[kichi] 内嵌播放器初始化失败：{reason}");
            notify(StartEvent::Failed(format!(
                "内嵌播放器初始化失败：{reason}"
            )));
            return;
        }
    };
    if let Err(err) = mpv.command("loadfile", &[url, "replace"]) {
        eprintln!("[kichi] 内嵌播放器加载失败：{}", describe_error(&err));
        notify(StartEvent::Failed(failure_message(
            title,
            &describe_error(&err),
        )));
        terminate(mpv);
        return;
    }
    let inhibitor = ScreenInhibitor::new();

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
    let mut reached_eof = false;
    let mut next_sub_id = 0u64;
    let mut selected = false;
    let mut late_held: Vec<SubtitleAssets> = Vec::new();
    let mut load_attempts = 0usize;
    let mut http_error: Option<String> = None;
    // mpv 的错误日志是「为什么打不开」最直接的证据；失败提示会带上它。
    let mut log_hint: Option<String> = None;

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
                Control::SetSpeed(speed) => {
                    if let Err(err) = mpv.set_property("speed", speed) {
                        eprintln!("[kichi] 内嵌播放器改倍速失败：{err}");
                    }
                    session.set_speed(speed);
                }
                Control::SelectTrack { audio, id } => {
                    let name = if audio { "aid" } else { "sid" };
                    let result = if id < 0 {
                        mpv.set_property(name, String::from("no"))
                    } else {
                        mpv.set_property(name, id)
                    };
                    if let Err(err) = result {
                        eprintln!("[kichi] 内嵌播放器切换轨道失败（{name}={id}）：{err}");
                    }
                    session.set_tracks(track_snapshot(&mpv));
                }
                Control::Keypress(key) => {
                    if let Err(err) = mpv.command("keypress", &[&key]) {
                        eprintln!("[kichi] 内嵌播放器按键「{key}」失败：{err}");
                    }
                    // mpv 的键位可能切轨 / 改倍速：刷新快照，界面下次打开菜单就是新状态。
                    session.set_tracks(track_snapshot(&mpv));
                }
                Control::SubtitleStyle(style) => apply_subtitle_style(&mpv, &style),
                Control::SetColorTarget(target) => {
                    if let Some(target) = color::target_update(current_target, target) {
                        apply_color_target(&mpv, target);
                        current_target = Some(target);
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
                        match attach_subtitles(
                            &mpv,
                            &assets,
                            &mut next_sub_id,
                            &settings.sub_name_tags,
                            &mut selected,
                        ) {
                            None => {
                                eprintln!("[kichi] 字幕：播放中补挂 {} 条", assets.tracks().len())
                            }
                            Some(note) => eprintln!("[kichi] 字幕：播放中补挂失败：{note}"),
                        }
                        session.set_tracks(track_snapshot(&mpv));
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
                    attach_subtitles(
                        &mpv,
                        assets,
                        &mut next_sub_id,
                        &settings.sub_name_tags,
                        &mut selected,
                    )
                });
                if let Some(resume) = resume {
                    let target = format!("{:.3}", resume.seconds as f64);
                    if let Err(err) = mpv.command("seek", &[&target, "absolute"]) {
                        eprintln!("[kichi] 恢复播放进度失败（只记日志）：{err}");
                    }
                }
                session.set_tracks(track_snapshot(&mpv));
                session.set_chapters(chapter_snapshot(&mpv));
                ready = true;
                inhibitor.set_active(!session.is_paused());
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
                ("pause", PropertyData::Flag(paused)) => {
                    session.set_paused(paused);
                    // 和外部 mpv 的 stop-screensaver=yes 一致：暂停即放开抑制。
                    if ready {
                        inhibitor.set_active(!paused);
                    }
                }
                // mpv 原生键位（↑↓ / m / `[` `]`）也能调音量、静音、倍速：
                // 同步回会话，界面据此显示（键位交给 mpv 时尤其重要）。
                ("volume", PropertyData::Double(volume)) => {
                    session.set_volume(volume.round().clamp(0.0, 100.0) as i32);
                }
                ("mute", PropertyData::Flag(muted)) => session.set_muted(muted),
                ("speed", PropertyData::Double(speed)) => session.set_speed(speed),
                // 缓冲 / 跳转 / 起播前核心空闲：界面据此显示加载环，而不是停在旧画面。
                ("paused-for-cache", PropertyData::Flag(buffering)) => {
                    session.set_paused_for_cache(buffering);
                }
                ("seeking", PropertyData::Flag(seeking)) => session.set_seeking(seeking),
                ("core-idle", PropertyData::Flag(idle)) => session.set_core_idle(idle),
                ("vo-configured", PropertyData::Flag(true)) => {
                    if !session.wayland_connected() {
                        // VO 配置成功却没走过内嵌代理：mpv 连到了别的地方，
                        // 画面不可能进 kichi 窗口。中止，绝不留黑浮层。
                        failure = Some(failure_message(
                            title,
                            "播放器没有接管内嵌画面（渲染器异常）",
                        ));
                        break;
                    }
                    guard.take();
                }
                _ => {}
            },
            Some(Ok(Event::LogMessage { prefix, text, .. })) => {
                // HTTP 状态码只出现在日志里；只留状态码，不碰后面的 URL。
                if let Some(reason) = crate::player::ipc::http_failure_from_text(text) {
                    http_error = Some(reason);
                }
                // 起播前的错误日志才归因于这一次加载；就绪后的日志（字幕 /
                // 音频设备等）与「为什么打不开」无关。
                if !ready {
                    eprintln!("[kichi] mpv {prefix}: {}", sanitize_log_line(text));
                    if let Some(hint) = hint_from_log(text) {
                        log_hint = Some(hint);
                    }
                }
            }
            Some(Ok(Event::EndFile(reason))) => {
                if reason == libmpv2_sys::mpv_end_file_reason_MPV_END_FILE_REASON_EOF {
                    reached_eof = true;
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
                    log_hint = None;
                    if !crate::session::download::sleep_cancellable(session.cancel(), delay) {
                        break;
                    }
                    if let Err(err) = mpv.command("loadfile", &[url, "replace"]) {
                        failure = Some(failure_message(
                            title,
                            &format!("无法重新加载：{}", describe_error(&err)),
                        ));
                        break;
                    }
                    continue;
                }
                if !ready && reason == libmpv2_sys::mpv_end_file_reason_MPV_END_FILE_REASON_ERROR {
                    let reason = match http_error.take() {
                        Some(detail) => format!("链接加载失败（HTTP {detail}）"),
                        None => "链接加载失败".to_string(),
                    };
                    failure = Some(failure_message(title, &reason));
                }
                break;
            }
            Some(Ok(Event::Shutdown)) => break,
            Some(Ok(_)) => {}
            Some(Err(err)) => {
                // libmpv2 的 wait_event 会把 `END_FILE(reason=error)` 的非零
                // error 直接转成 Err（见该 crate 的实现），所以真正的「打不开
                // 链接」都落在这一支，而不是上面的 EndFile 分支。
                if !ready
                    && is_retryable_load_error(&err)
                    && load_attempts < crate::player::ipc::LOAD_RETRIES
                {
                    let delay = crate::player::ipc::retry_delay(load_attempts);
                    load_attempts += 1;
                    eprintln!(
                        "[kichi] 内嵌播放加载失败（{}，第 {load_attempts} 次），{delay:?} 后用同一链接重试",
                        describe_error(&err)
                    );
                    http_error = None;
                    log_hint = None;
                    if !crate::session::download::sleep_cancellable(session.cancel(), delay) {
                        break;
                    }
                    if let Err(err) = mpv.command("loadfile", &[url, "replace"]) {
                        failure = Some(failure_message(
                            title,
                            &format!("无法重新加载：{}", describe_error(&err)),
                        ));
                        break;
                    }
                    continue;
                }
                if !ready {
                    let reason = failure_reason(&err, http_error.take(), log_hint.take());
                    // 界面上给人话，stderr 保留原始错误码，方便对照 mpv 文档排查。
                    eprintln!("[kichi] 内嵌播放失败原因：{reason}（mpv: {err:?}）");
                    failure = Some(failure_message(title, &reason));
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
            end: if reached_eof {
                crate::player::EndReason::Eof
            } else if session.cancel().is_cancelled() {
                crate::player::EndReason::Stopped
            } else {
                crate::player::EndReason::Unknown
            },
        });
    } else if let Some(failure) = failure {
        notify(StartEvent::Failed(failure));
    } else if session.cancel().is_cancelled() {
        notify(StartEvent::Cancelled);
    } else if let Some(reason) = session.proxy_error() {
        notify(StartEvent::Failed(failure_message(title, &reason)));
    } else {
        notify(StartEvent::Failed(failure_message(title, "播放提前结束")));
    }

    drop(late_held);
    drop(late);
    drop(subtitles);
    terminate(mpv);
}

/// 失败提示条文案：原因在前，文件名在后。
///
/// 提示条单行显示、右侧截断；文件名往往很长，放后面才不会把真正的原因挤掉。
fn failure_message(title: &str, reason: &str) -> String {
    format!("内嵌播放器播放失败：{reason}（「{title}」）")
}

/// 一次失败的原因：HTTP 状态最具体，其次是 mpv 的日志，最后按错误码兜底。
fn failure_reason(err: &Error, http: Option<String>, hint: Option<String>) -> String {
    if let Some(status) = http {
        return format!("链接加载失败（HTTP {status}）");
    }
    hint.unwrap_or_else(|| describe_error(err))
}

/// 把 libmpv 的错误码翻译成用户能看懂的原因。
///
/// 直接显示 `Error::Display` 只会得到 `Raw(-16)` 这样的调试信息。
fn describe_error(err: &Error) -> String {
    match err {
        Error::Raw(code) => match *code {
            mpv_error::LoadingFailed => {
                "链接加载失败（打不开链接，或服务器拒绝了请求）".to_string()
            }
            mpv_error::AoInitFailed => "音频输出初始化失败（检查系统音频设备）".to_string(),
            mpv_error::VoInitFailed => "视频输出初始化失败（检查显卡驱动或渲染后端）".to_string(),
            mpv_error::NothingToPlay => {
                "没有可播放的音视频流（文件为空、已损坏，或链接返回的不是视频）".to_string()
            }
            mpv_error::UnknownFormat => {
                "无法识别文件格式（链接可能已过期，或文件已损坏）".to_string()
            }
            mpv_error::Unsupported => "当前环境不支持所需的播放功能".to_string(),
            mpv_error::NotImplemented => "当前播放器不支持这项功能".to_string(),
            other => format!(
                "播放器错误（{}，代码 {other}）",
                libmpv2_sys::mpv_error_str(*code)
            ),
        },
        Error::VersionMismatch { linked, loaded } => {
            format!("libmpv 版本不匹配（链接 {linked}，加载 {loaded}）")
        }
        Error::InvalidUtf8 => "播放器返回了无效文本".to_string(),
        Error::Null => "播放器返回了空结果".to_string(),
    }
}

/// 只有「可能只是 CDN 暂时没准备好」的加载错误才值得用同一链接重试。
fn is_retryable_load_error(err: &Error) -> bool {
    matches!(
        err,
        Error::Raw(code)
            if matches!(
                *code,
                mpv_error::LoadingFailed | mpv_error::NothingToPlay | mpv_error::UnknownFormat
            )
    )
}

/// 从 mpv 的错误日志里认出常见原因。
///
/// 只映射已知模式，不把原始日志直接塞进界面：日志里可能带签名直链。
/// 未识别的日志仍然用 [`sanitize_log_line`] 打到 stderr，方便排查。
fn hint_from_log(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    // Flatpak 运行时自带的 ffmpeg 是精简版，h264 / hevc / eac3 等编码需要
    // codecs-extra 扩展；这条日志就是最直接的证据。
    if lower.contains("failed to initialize a decoder for codec")
        || lower.contains("failed to open codec")
        || lower.contains("decoder not found")
    {
        return Some("缺少可用的音视频解码器（Flatpak 版请安装 codecs-extra 扩展）".to_string());
    }
    let hint = if lower.contains("failed to recognize file format")
        || lower.contains("unrecognized file format")
        || lower.contains("could not detect file format")
    {
        "无法识别文件格式（链接可能已过期，或服务器返回的不是视频内容）"
    } else if lower.contains("invalid data found when processing input") {
        "文件内容无效或已损坏"
    } else if lower.contains("moov atom not found") || lower.contains("failed to read header") {
        "文件不完整或已损坏"
    } else if lower.contains("could not find codec parameters") {
        "找不到可用的音视频编码信息（文件可能不完整）"
    } else if lower.contains("no audio or video streams") {
        "文件里没有音视频流"
    } else if lower.contains("could not open codec") || lower.contains("unsupported codec") {
        "缺少可用的解码器（当前播放器不支持这个视频编码）"
    } else if lower.contains("protocol not found") {
        "播放器不支持这个链接的协议"
    } else {
        return None;
    };
    Some(hint.to_string())
}

/// 日志脱敏：带协议的地址只留主机名，去掉控制字符并截断。
fn sanitize_log_line(text: &str) -> String {
    let mut out = String::new();
    for token in text.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        if token.contains("://") {
            out.push_str(&crate::player::mask_url(token));
        } else {
            out.extend(token.chars().filter(|ch| !ch.is_control()));
        }
        if out.chars().count() >= 200 {
            break;
        }
    }
    out.chars().take(200).collect()
}

/// 按合成器首选色彩（已归一化）决定 mpv 的 output target。
///
/// SDR / HDR 都用同一个 target；HDR 源由 mpv tone-map 到它。返回创建时
/// 实际采用的 target，播放中换屏后由 [`Control::SetColorTarget`] 更新。
fn create_mpv(
    title: &str,
    settings: &SessionSettings,
) -> Result<(Mpv, Option<color::ColorTarget>), libmpv2::Error> {
    let user_conf = settings.conf.as_deref().map(conf::Parsed::load);
    // 合成器首选色彩；播放窗口出现在另一个 output 后由 C++ presenter 更新。
    let preferred = color::preferred_target();
    let mpv = Mpv::with_initializer(|init| {
        // 先应用用户配置，再写内嵌固定项：固定的 vo / gpu / 输入配置永远赢。
        if let Some(parsed) = &user_conf {
            for (name, value) in &parsed.options {
                // 单条写错（未知选项 / 非法值）只跳过，不阻断播放。
                if let Err(err) = init.set_option(name.as_str(), value.as_str()) {
                    eprintln!("[kichi] embed-mpv.conf 忽略选项「{name}」：{err}");
                }
            }
        }
        let (gpu_context, gpu_api) = gpu_backend(settings.vulkan);
        for (name, value) in [
            ("vo", "gpu-next"),
            ("gpu-context", gpu_context),
            ("gpu-api", gpu_api),
            // mpv 的 surface 是代理造的，image description 由 kichi 设到真
            // surface 上。让 mpv 自己发 colorspace hint 时，Vulkan/Wayland
            // 没有 color surface 会把 target 改回 sRGB，导致 target-* 被忽略、
            // 画面用 sRGB 编码却贴着 PQ 描述（非常暗）。禁用 hint，让
            // target-* 真正决定渲染目标。
            ("target-colorspace-hint", "no"),
            ("hwdec", "auto-safe"),
            // 测试自检时把 mpv 日志打到 stderr，方便排查链路；正式构建保持静默。
            ("terminal", if cfg!(test) { "yes" } else { "no" }),
            // 错误级日志仍要生成：HTTP 403 这类状态码只出现在日志里。
            ("msg-level", if cfg!(test) { "all=v" } else { "all=error" }),
            // 剪贴板只走 VO：默认的 wayland 后端会抢在 VO 之前用掉 WAYLAND_SOCKET。
            ("clipboard-backends", "vo"),
            // 默认键位始终加载：播放窗口把没被 kichi 接住的按键经 `keypress` 转给 mpv。
            // kichi 自己的 embed-input.conf 叠加在上面；不读用户 input.conf / 脚本。
            ("input-default-bindings", "yes"),
            // 键盘事件由 kichi 的 Qt 窗口转发；mpv 自己的 VO 不直接抢焦点。
            ("input-vo-keyboard", "no"),
            ("osc", "no"),
            ("osd-level", "0"),
            ("config", "no"),
            ("load-scripts", "no"),
            ("idle", "yes"),
            ("force-window", "yes"),
            // 屏幕抑制由 kichi 自己持有：内嵌 mpv 的 surface 不交给合成器，
            // 它的 idle-inhibit 建立不起来（见 player/screensaver.rs）。
            ("stop-screensaver", "no"),
            ("ytdl", "no"),
            ("audio-display", "no"),
            ("sub-auto", "no"),
            // 播放窗口由 kichi 决定尺寸：mpv 必须按 configure 的整窗出帧，
            // 由它在 buffer 内部留黑边；否则 mpv 会为保持视频比例自行改变
            // 窗口尺寸（如 640x360 被改成 640x1138），C++ 再按整窗拉伸 viewport
            // 就会把画面压扁 / 拉长。
            ("keepaspect-window", "no"),
        ] {
            if let Err(err) = init.set_option(name, value) {
                eprintln!("[kichi] 内嵌播放器固定选项「{name}={value}」设置失败：{err}");
                return Err(err);
            }
        }
        // 一次性设好归一化后的输出 target；HDR 源也会被 mpv tone-map 到
        // 这个 target，和外部 mpv 的 target-colorspace-hint 行为一致。
        if let Some(target) = preferred {
            if let Some(primaries) = color::primaries_name(target.primaries) {
                init.set_option("target-prim", primaries)?;
            }
            if let Some(transfer) = color::transfer_name(target.transfer) {
                init.set_option("target-trc", transfer)?;
            }
            if target.max_luminance > 0 {
                init.set_option("target-peak", target.max_luminance.to_string().as_str())?;
            }
        }
        // 键位文件交给 mpv 解析；文件不存在时不设置，免得每次开播都报读文件失败。
        if let Some(input_conf) = &settings.input_conf {
            if input_conf.is_file() {
                let path = input_conf.to_string_lossy();
                init.set_option("input-conf", path.as_ref())?;
            }
        }
        // 首选语言交给 mpv，加载文件时自动选轨；空串表示不指定，保持 mpv 默认。
        if !settings.audio_lang.is_empty() {
            init.set_option("alang", settings.audio_lang.as_str())?;
        }
        if !settings.sub_lang.is_empty() {
            init.set_option("slang", settings.sub_lang.as_str())?;
        }
        Ok(())
    })?;
    if let Some(parsed) = &user_conf {
        eprintln!(
            "[kichi] embed-mpv.conf 已读取 {} 项（忽略内嵌必需项 {} 项）",
            parsed.options.len(),
            parsed.ignored
        );
        for warning in &parsed.warnings {
            eprintln!("[kichi] embed-mpv.conf：{warning}");
        }
    }
    mpv.set_property("force-media-title", title)?;
    // 音量 / 静音 / 倍速 / 字幕样式在建 mpv 时一次设好，避免首帧先按默认值闪一下。
    mpv.set_property("volume", i64::from(settings.volume))?;
    mpv.set_property("mute", settings.muted)?;
    mpv.set_property("speed", settings.speed)?;
    apply_subtitle_style(&mpv, &settings.subtitles);
    // 自动重试与错误提示都要读 HTTP 状态码；请求错误级日志。
    // SAFETY: ctx 来自刚创建好的 libmpv2 句柄，字符串静态且以 NUL 结尾。
    unsafe {
        libmpv2_sys::mpv_request_log_messages(mpv.ctx.as_ptr(), c"error".as_ptr());
    }
    for (index, (name, format)) in [
        ("time-pos", Format::Double),
        ("duration", Format::Double),
        ("pause", Format::Flag),
        ("volume", Format::Double),
        ("mute", Format::Flag),
        ("speed", Format::Double),
        ("vo-configured", Format::Flag),
        // 缓冲提示：等缓存 / 跳转中 / 起播前核心空闲（与手动暂停区分）。
        ("paused-for-cache", Format::Flag),
        ("seeking", Format::Flag),
        ("core-idle", Format::Flag),
    ]
    .into_iter()
    .enumerate()
    {
        mpv.observe_property(name, format, index as u64 + 1)?;
    }
    Ok((mpv, preferred))
}

/// 把首选 target 写进 mpv；单项失败只记日志，不中断播放。
fn apply_color_target(mpv: &Mpv, target: color::ColorTarget) {
    let set = |name: &str, result: Result<(), libmpv2::Error>| {
        if let Err(err) = result {
            eprintln!("[kichi] 内嵌播放器设置 {name} 失败：{err}");
        }
    };
    if let Some(primaries) = color::primaries_name(target.primaries) {
        set("target-prim", mpv.set_property("target-prim", primaries));
    }
    if let Some(transfer) = color::transfer_name(target.transfer) {
        set("target-trc", mpv.set_property("target-trc", transfer));
    }
    if target.max_luminance > 0 {
        set(
            "target-peak",
            mpv.set_property("target-peak", i64::from(target.max_luminance)),
        );
    }
    eprintln!(
        "[kichi] 内嵌播放器色彩 target 更新：primaries={} transfer={} peak={}",
        color::primaries_name(target.primaries).unwrap_or("unknown"),
        color::transfer_name(target.transfer).unwrap_or("unknown"),
        target.max_luminance
    );
}

/// 内嵌渲染后端（`gpu-context` + `gpu-api`）。
///
/// 默认跟随设置（Vulkan）；`KICHI_EMBED_GPU` 可强制覆盖，方便对比与排查。
fn gpu_backend(use_vulkan: bool) -> (&'static str, &'static str) {
    let override_value = std::env::var("KICHI_EMBED_GPU").ok();
    let backend = parse_gpu_backend(override_value.as_deref(), use_vulkan);
    if let Some(value) = override_value {
        if !value.eq_ignore_ascii_case("vulkan") && !value.eq_ignore_ascii_case("opengl") {
            eprintln!(
                "[kichi] 无法识别 KICHI_EMBED_GPU={value}，按设置使用 {}",
                backend.1
            );
        }
    }
    eprintln!("[kichi] 内嵌播放渲染后端：{}", backend.1);
    backend
}

fn parse_gpu_backend(
    override_value: Option<&str>,
    use_vulkan: bool,
) -> (&'static str, &'static str) {
    let vulkan = match override_value {
        Some(value) if value.eq_ignore_ascii_case("vulkan") => true,
        Some(value) if value.eq_ignore_ascii_case("opengl") => false,
        _ => use_vulkan,
    };
    if vulkan {
        ("waylandvk", "vulkan")
    } else {
        ("wayland", "opengl")
    }
}

/// 读一遍 mpv 的轨道列表（音轨 + 字幕轨）。
///
/// libmpv 没有公开 node 取值接口，只能按 `track-list/N/<field>` 逐项查；
/// 轨道数很少（个位数），在加载 / 切轨 / 按键之后刷新一次足够。
fn track_snapshot(mpv: &Mpv) -> Vec<TrackInfo> {
    let count: i64 = mpv.get_property("track-list/count").unwrap_or(0);
    let mut tracks = Vec::new();
    for index in 0..count {
        let Ok(kind) = mpv.get_property::<String>(&format!("track-list/{index}/type")) else {
            continue;
        };
        let audio = match kind.as_str() {
            "audio" => true,
            "sub" => false,
            _ => continue,
        };
        let Ok(id) = mpv.get_property::<i64>(&format!("track-list/{index}/id")) else {
            continue;
        };
        let title: String = mpv
            .get_property(&format!("track-list/{index}/title"))
            .unwrap_or_default();
        let lang: String = mpv
            .get_property(&format!("track-list/{index}/lang"))
            .unwrap_or_default();
        let codec: String = mpv
            .get_property(&format!("track-list/{index}/codec"))
            .unwrap_or_default();
        let selected: bool = mpv
            .get_property(&format!("track-list/{index}/selected"))
            .unwrap_or(false);
        let label = if !title.trim().is_empty() {
            title
        } else if !lang.trim().is_empty() {
            lang.clone()
        } else if !codec.trim().is_empty() {
            codec
        } else {
            format!("{} {}", if audio { "音轨" } else { "字幕" }, index + 1)
        };
        tracks.push(TrackInfo {
            id,
            audio,
            label,
            lang,
            selected,
        });
    }
    tracks
}

/// 读一遍章节（起点 + 标题）。和轨道一样按 `chapter-list/N/<field>` 逐项查；
/// 章节是容器元数据，文件加载完就不会变，读一次足够。
fn chapter_snapshot(mpv: &Mpv) -> Vec<ChapterInfo> {
    let count: i64 = mpv.get_property("chapter-list/count").unwrap_or(0);
    (0..count)
        .filter_map(|index| {
            let time = mpv
                .get_property::<f64>(&format!("chapter-list/{index}/time"))
                .ok()
                .and_then(valid_seconds)?;
            let title: String = mpv
                .get_property(&format!("chapter-list/{index}/title"))
                .unwrap_or_default();
            Some(ChapterInfo { time, title })
        })
        .collect()
}

/// 套用字幕样式；单个属性失败只记日志，其余继续。
fn apply_subtitle_style(mpv: &Mpv, style: &SubtitleStyle) {
    let set = |name: &str, result: Result<(), libmpv2::Error>| {
        if let Err(err) = result {
            eprintln!("[kichi] 内嵌播放器设置字幕 {name} 失败：{err}");
        }
    };
    set(
        "sub-font-size",
        mpv.set_property("sub-font-size", style.font_size),
    );
    set("sub-pos", mpv.set_property("sub-pos", style.position));
    set(
        "sub-color",
        mpv.set_property("sub-color", style.color.clone()),
    );
    set(
        "sub-outline-size",
        mpv.set_property("sub-outline-size", style.outline_size),
    );
    set(
        "sub-outline-color",
        mpv.set_property("sub-outline-color", style.outline_color.clone()),
    );
}

/// 挂字幕；返回一句失败原因（成功 / 没有字幕时 `None`）。
///
/// 首选语言已在下载阶段排到最前；这里只在「当前没有选中任何字幕」时 `select`
/// 第一条。若内封字幕已经选中且语言命中首选，外挂字幕只挂不选，避免抢掉用户
/// 想要的内封字幕。
fn attach_subtitles(
    mpv: &Mpv,
    assets: &SubtitleAssets,
    _next_sub_id: &mut u64,
    preferred: &[String],
    selected: &mut bool,
) -> Option<String> {
    let mut select_next = !*selected && !selected_subtitle_matches(mpv, preferred);
    for track in assets.tracks() {
        let path = track.path.to_string_lossy();
        let flag = if select_next { "select" } else { "auto" };
        if let Err(err) = mpv.command("sub-add", &[&path, flag, &track.title]) {
            eprintln!("[kichi] 字幕：sub-add 失败：{err}");
            return Some(format!("字幕未能加载：{err}"));
        }
        if select_next {
            select_next = false;
            *selected = true;
        }
    }
    if assets.is_empty() {
        None
    } else {
        eprintln!("[kichi] 字幕：已挂载 {} 条", assets.tracks().len());
        None
    }
}

fn selected_subtitle_matches(mpv: &Mpv, preferred: &[String]) -> bool {
    if preferred.is_empty() {
        return false;
    }
    track_snapshot(mpv).into_iter().any(|track| {
        !track.audio
            && track.selected
            && kichi_core::subtitles::matches_language(&track.lang, preferred)
    })
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

#[cfg(test)]
mod tests {
    use super::{
        describe_error, failure_message, failure_reason, hint_from_log, is_retryable_load_error,
        parse_gpu_backend, sanitize_log_line,
    };
    use libmpv2::{mpv_error, Error};

    #[test]
    fn gpu_backend_prefers_setting_and_env_override() {
        assert_eq!(parse_gpu_backend(None, true), ("waylandvk", "vulkan"));
        assert_eq!(parse_gpu_backend(None, false), ("wayland", "opengl"));
        assert_eq!(
            parse_gpu_backend(Some("opengl"), true),
            ("wayland", "opengl")
        );
        assert_eq!(
            parse_gpu_backend(Some("Vulkan"), false),
            ("waylandvk", "vulkan")
        );
        assert_eq!(
            parse_gpu_backend(Some("metal"), false),
            ("wayland", "opengl")
        );
    }

    #[test]
    fn raw_error_codes_become_readable_chinese() {
        // 防回归：界面不再显示 Raw(-16) 这种调试信息。
        let nothing = Error::Raw(mpv_error::NothingToPlay);
        let text = describe_error(&nothing);
        assert!(text.contains("音视频流"), "{text}");
        assert!(!text.contains("Raw"), "{text}");
        assert!(is_retryable_load_error(&nothing));

        assert!(!is_retryable_load_error(&Error::Raw(
            mpv_error::VoInitFailed
        )));
        assert!(describe_error(&Error::Raw(mpv_error::VoInitFailed)).contains("视频输出"));
        assert!(describe_error(&Error::Raw(mpv_error::AoInitFailed)).contains("音频输出"));
    }

    #[test]
    fn failure_message_puts_the_reason_before_the_title() {
        // 提示条会在右侧截断；原因放前面才看得到。
        let text = failure_message("很长的文件名.S02E04.1080p.mkv", "无法识别文件格式");
        assert!(
            text.starts_with("内嵌播放器播放失败：无法识别文件格式"),
            "{text}"
        );
        assert!(text.contains("很长的文件名"), "{text}");
    }

    #[test]
    fn http_status_wins_over_more_generic_hints() {
        let err = Error::Raw(mpv_error::LoadingFailed);
        let text = failure_reason(
            &err,
            Some("403 Forbidden".into()),
            Some("无法识别文件格式".into()),
        );
        assert_eq!(text, "链接加载失败（HTTP 403 Forbidden）");
    }

    #[test]
    fn known_mpv_log_lines_become_hints() {
        assert!(hint_from_log("Failed to recognize file format.")
            .unwrap()
            .contains("无法识别文件格式"));
        assert!(
            hint_from_log("[ffmpeg] Could not open codec: Invalid argument")
                .unwrap()
                .contains("解码器")
        );
        // Flatpak 精简版 ffmpeg 的实际日志（真实复现过）。
        let flatpak =
            hint_from_log("vd: Failed to initialize a decoder for codec 'h264'.").unwrap();
        assert!(flatpak.contains("codecs-extra"), "{flatpak}");
        assert_eq!(hint_from_log("some unrelated error"), None);
    }

    #[test]
    fn log_sanitizer_keeps_host_but_drops_signed_url() {
        let line =
            "http: HTTP error 403 Forbidden from https://cdn.example.com/a.mkv?sign=SECRET&x=1";
        let clean = sanitize_log_line(line);
        assert!(clean.contains("cdn.example.com/…"), "{clean}");
        assert!(!clean.contains("SECRET"), "{clean}");
    }
}
