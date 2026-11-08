//! 播放器启动与播放事件：外部播放器拉起、内嵌窗口事件、窗口关闭收尾。

use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;
use kichi_core::PlayOption;

use super::actions::enqueue_ui;
use super::play::wants_embedded;
use super::qobject;
use crate::player::subtitles::{LateSubtitles, SubtitleAssets};
use crate::player::{self, mask_url, PlaybackExtras, PlayerConfig, StartEvent};

fn event_notify(
    playback_id: String,
    event_notice: Option<String>,
    epoch: u64,
    report_history: bool,
) -> impl Fn(StartEvent) + Send + Sync + 'static {
    move |event| {
        let playback_id = playback_id.clone();
        let event_notice = event_notice.clone();
        enqueue_ui(move |mut qichi| {
            qichi
                .as_mut()
                .on_play_event(event, event_notice, &playback_id, epoch, report_history);
        });
    }
}

impl qobject::Kichi {
    /// 启动外部播放器（URL 过期时先重取一次）。
    ///
    /// 成功后**保持** `playLoading`，等 `player::start` 回投「窗口已出现」。
    // 参数共同描述一次播放启动，拆结构只会让调用点来回打包。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn start_player(
        mut self: Pin<&mut Self>,
        option: &PlayOption,
        id: &str,
        title: &str,
        epoch: u64,
        notice: Option<String>,
        subtitles: Option<SubtitleAssets>,
        late: Option<LateSubtitles>,
        resume: Option<kichi_core::PlaybackProgress>,
        report_history: bool,
    ) {
        if option.is_expired(None) {
            self.as_mut()
                .fail_play_loading("播放链接已过期，请重新点播放");
            return;
        }
        let settings = self.settings.clone();
        let (config, player_notice) = PlayerConfig::resolve_playable(
            settings.effective_player_backend(),
            settings.player_command_override(),
            settings.player_args_override(),
        );
        let cancel = self.as_ref().rust().play_cancel.clone();
        let track_count = subtitles.as_ref().map_or(0, |assets| assets.tracks().len());
        // 期间按了取消 / 换了片子：加载态已经被清掉，这条回投不能再启动播放器
        let Some(cancel) = cancel else {
            eprintln!("[kichi] 字幕：播放已收尾，丢弃 {track_count} 条字幕");
            return;
        };
        if cancel.is_cancelled() || self.as_ref().rust().play_epoch != epoch {
            eprintln!("[kichi] 字幕：播放已取消 / 过期，丢弃 {track_count} 条字幕");
            return;
        }
        if track_count > 0 {
            eprintln!("[kichi] 字幕：把 {track_count} 条字幕交给播放器");
        }
        let playback_id = id.to_owned();
        let mut fallback_notice = None;
        if wants_embedded(&settings) {
            match crate::player::embed::available() {
                Ok(()) => {
                    let event_notice = super::play_prepare::combine_notes(notice, player_notice);
                    let notify = event_notify(playback_id, event_notice, epoch, report_history);
                    let audio_tags = settings.player_audio_lang.tags();
                    let sub_tags = settings.player_sub_lang.tags();
                    let mut session_settings = crate::player::embed::SessionSettings::new(
                        settings.player_volume,
                        settings.player_muted,
                        f64::from(settings.player_speed_percent) / 100.0,
                        crate::player::embed::SubtitleStyle::new(
                            i32::from(settings.player_sub_font_size),
                            i32::from(settings.player_sub_pos),
                            &settings.player_sub_color,
                            i32::from(settings.player_sub_outline_size),
                            &settings.player_sub_outline_color,
                        ),
                        audio_tags.mpv_value().unwrap_or_default(),
                        sub_tags.mpv_value().unwrap_or_default(),
                        sub_tags.names,
                    );
                    if settings.player_mpv_conf {
                        session_settings.conf = Some(crate::session::paths::embed_mpv_conf_path());
                    }
                    if settings.player_input_conf {
                        session_settings.input_conf =
                            Some(crate::session::paths::embed_input_conf_path());
                    }
                    session_settings.vulkan = settings.player_vulkan;
                    let started = crate::player::embed::start(
                        &option.url,
                        title,
                        cancel,
                        PlaybackExtras {
                            subtitles,
                            late,
                            resume,
                        },
                        session_settings,
                        notify,
                    );
                    match started {
                        Ok(_) => {
                            {
                                let mut rust = self.as_mut().rust_mut();
                                // 队列切集保持窗口不关；普通新开播仍先收起，等 Ready。
                                if !rust.play_queue_state.advancing {
                                    rust.embed_active = false;
                                }
                                rust.embed_title = QString::from(title);
                                rust.embed_pending_epoch = Some(epoch);
                            }
                            self.as_mut().notify();
                            eprintln!(
                                "[kichi] 内嵌播放「{title}」{} → {}",
                                option.label,
                                mask_url(&option.url)
                            );
                        }
                        Err(err) => {
                            self.as_mut().stop_embed_and_clear();
                            self.as_mut().fail_play_loading(&err);
                        }
                    }
                    return;
                }
                Err(reason) => {
                    self.as_mut().stop_embed_and_clear();
                    if crate::platform::flatpak_sandbox() {
                        // 沙箱里没有外部播放器可回退；这里报清楚，别让用户
                        // 看到「已改用外部播放器」之后又找不到 mpv。
                        self.as_mut()
                            .fail_play_loading(&format!("Flatpak 版只提供内嵌播放器：{reason}"));
                        return;
                    }
                    fallback_notice = Some(format!("内嵌播放不可用（{reason}），已改用外部播放器"));
                }
            }
        } else {
            self.as_mut().stop_embed_and_clear();
        }

        let event_notice = super::play_prepare::combine_notes(
            super::play_prepare::combine_notes(notice, player_notice),
            fallback_notice,
        );
        let started = player::start(
            &config,
            &option.url,
            title,
            cancel,
            PlaybackExtras {
                subtitles,
                late,
                resume,
            },
            event_notify(playback_id, event_notice, epoch, report_history),
        );
        eprintln!(
            "[kichi] 播放「{title}」{} → {}",
            option.label,
            mask_url(&option.url)
        );
        if let Err(err) = started {
            self.as_mut().fail_play_loading(&err);
        }
    }

    /// 一次播放的结果（后台线程回投）。
    fn on_play_event(
        mut self: Pin<&mut Self>,
        event: StartEvent,
        notice: Option<String>,
        id: &str,
        epoch: u64,
        report_history: bool,
    ) {
        // 已经切到下一次播放：旧会话的失败 / 取消不再打扰当前界面；
        // 但旧播放器真正退出时仍要清掉它自己的「播放中」标记。
        if self.as_ref().rust().play_epoch != epoch
            && !matches!(
                event,
                StartEvent::Progress { .. } | StartEvent::Stopped { .. }
            )
        {
            return;
        }
        match event {
            StartEvent::Ready { subtitle_note } => {
                if self.as_ref().rust().play_epoch != epoch {
                    return;
                }
                self.as_mut().update_active_play_session(epoch, id, true);
                let notice =
                    super::play_prepare::combine_notes(notice, subtitle_note).unwrap_or_default();
                {
                    let mut rust = self.as_mut().rust_mut();
                    rust.play_loading = false;
                    rust.play_loading_id = QString::default();
                    rust.play_loading_name = QString::default();
                    rust.play_cancel = None;
                    rust.play_error = QString::default();
                    rust.play_notice = QString::from(notice.as_str());
                    // 内嵌会话出图：这时才把整窗视频浮层亮出来
                    if rust.embed_pending_epoch == Some(epoch) {
                        rust.embed_active = true;
                        rust.embed_live_epoch = Some(epoch);
                        // 同一个播放窗口切下一集：QML 据此刷新章节 / 进度。
                        rust.embed_serial = rust.embed_serial.wrapping_add(1);
                    }
                    // 成功后清掉旧提醒：不常驻「已用 … 播放…」
                    rust.status_message = QString::default();
                }
                self.finish_play();
            }
            StartEvent::Progress { progress } => {
                if report_history {
                    self.as_mut().queue_play_progress(id, epoch, progress);
                }
                self.as_mut().maybe_prefetch_next(progress, epoch);
            }
            StartEvent::Stopped { progress, end } => {
                if let Some(progress) = progress.filter(|_| report_history) {
                    self.as_mut().queue_play_progress(id, epoch, progress);
                }
                self.as_mut().update_active_play_session(epoch, id, false);
                {
                    let mut rust = self.as_mut().rust_mut();
                    if rust.embed_live_epoch == Some(epoch) {
                        rust.embed_live_epoch = None;
                    }
                }
                let advance = self.as_ref().rust().play_epoch == epoch
                    && super::playlist::should_auto_advance(
                        end,
                        self.as_ref().rust().auto_play_next,
                        self.as_ref().rust().play_queue_state.playlist.has_next(),
                    );
                if advance {
                    // 不收起内嵌窗口：下一集的 Ready 会把 embedSerial +1。
                    self.as_mut().start_queue_offset(1);
                    self.as_mut().notify();
                    return;
                }
                self.as_mut().clear_embed_state(epoch);
                self.as_mut().notify();
            }
            StartEvent::Failed(reason) => {
                self.as_mut().clear_embed_state(epoch);
                {
                    let mut rust = self.as_mut().rust_mut();
                    if rust.embed_live_epoch == Some(epoch) {
                        rust.embed_live_epoch = None;
                    }
                }
                self.as_mut().update_active_play_session(epoch, id, false);
                // `reason` 已经带了播放器上下文（…未能播放「title」：…），直接显示
                self.as_mut().fail_play_loading(&reason);
            }
            StartEvent::Cancelled => {
                self.as_mut().clear_embed_state(epoch);
                {
                    let mut rust = self.as_mut().rust_mut();
                    if rust.embed_live_epoch == Some(epoch) {
                        rust.embed_live_epoch = None;
                    }
                }
                self.as_mut().update_active_play_session(epoch, id, false);
                // 取消后不给任何文字反馈
                self.as_mut().hide_play_loading();
                self.finish_play();
            }
        }
    }
}
