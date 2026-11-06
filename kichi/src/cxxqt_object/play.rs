//! 播放：弹固定档位菜单 → 选中后取直链 → 用外部 mpv 串流。
//!
//! 菜单**不等网络**：四项（原画 / 1080P / 720P / 480P）是常量，点「播放」立刻弹出；
//! 用户在菜单里选了某一档，才发那一次「取直链」的请求（签名链接只能从服务端拿）。
//!
//! 直链只活在 Rust 侧的局部变量里，QML 拿到的菜单里**没有 URL**，
//! 也就没有把签名链接写进界面日志或剪贴板的机会。
//!
//! 等待过程中 `playLoading` + `playLoadingId` 指向当前文件，文件缩略图据此
//! 显示加载状态并提供取消；条目离开当前列表或离开对应页面时，
//! `PlayToast.qml` 接替显示取消入口。
//! 出窗口 / 失败 / 取消时两处状态一起收起。
//! 底栏（`statusMessage`）不参与加载提示；无提醒时它显示当前页面上下文。

use std::pin::Pin;
use std::sync::Arc;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QMap, QMapPair_QString_QVariant, QString, QStringList, QVariant};
use kichi_core::types::Tier;
use kichi_core::PlayOption;

use super::actions::{enqueue_ui, run_in_background};
use super::qobject;
use super::QVariantList;
use crate::player::subtitles::{LateSubtitles, SubtitleAssets};
use crate::player::{self, mask_url, PlaybackExtras, PlayerConfig, StartEvent};
use crate::session::download::Cancel;
use crate::session::subtitles::SubtitlePlan;
use crate::settings::{PlayerBackend, Settings};

type QVariantMap = QMap<QMapPair_QString_QVariant>;

/// 设置要求内嵌播放，且没有用 `KICHI_PLAYER` 临时改走外部命令。
fn wants_embedded(settings: &Settings) -> bool {
    settings.player_backend == PlayerBackend::Embedded && std::env::var_os("KICHI_PLAYER").is_none()
}

/// 播放事件回投：切回界面线程，带上这次播放的代数与提示文案。
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

/// 不可用原因 → 界面文案。
pub(crate) fn reason_text(reason: Option<&str>) -> String {
    match reason {
        Some(kichi_core::media::REASON_QUOTA) => "需要额外播放配额".to_owned(),
        Some(kichi_core::media::REASON_HIDDEN) => "官方未开放（已隐藏）".to_owned(),
        Some(kichi_core::media::REASON_NOT_READY) => "链接未就绪".to_owned(),
        Some(other) => other.to_owned(),
        None => String::new(),
    }
}

impl qobject::Kichi {
    /// 点「播放」：设置里选了具体档位就**直接**播那一档；否则立刻弹固定四项菜单。
    ///
    /// 两种路径都要发一次请求才能拿到签名直链；区别只是「弹出菜单前是否先等它」：
    /// 菜单内容不依赖请求结果，所以「每次询问」不等网络，点开就有。
    pub fn request_play(mut self: Pin<&mut Self>, id: QString) {
        let id = id.to_string();
        let Some(name): Option<String> = self.as_ref().rust().entry_name(&id) else {
            self.as_mut()
                .set_message("该文件已不在当前列表中，请刷新后重试");
            return;
        };
        self.as_mut().request_play_named(&id, &name);
    }

    /// 当前列表之外已知 id / 文件名的播放入口（离线任务产物走这里）。
    pub(crate) fn request_play_named(mut self: Pin<&mut Self>, id: &str, name: &str) {
        // 新的播放取消上一次「还在加载」的那次（已在播放的 mpv 不受影响）：
        // 否则它被 epoch 作废后，旧缩略图会永远停在「正在加载…」
        self.as_mut().cancel_play_loading();
        // 在点播放这一刻快照字幕计划：菜单开着时目录刷新不影响本次播放。
        // 两条路径（直接播 / 先弹菜单）都要有，所以放在分支之前。
        let plan = {
            let this = self.as_ref();
            let rust = this.rust();
            super::play_prepare::plan_for(id, &rust.entries, &rust.search_index)
        };
        self.as_mut().rust_mut().play_subtitle_plan = plan;
        // 设置里指定了档位：等同用户在菜单里点了那一项，不弹菜单
        if let Some(tier) = fixed_tier_of(self.settings.preferred_quality) {
            self.as_mut().prepare_play(id, name);
            self.as_mut().fetch_and_play(tier, name);
            return;
        }
        let mut menu = QVariantList::default();
        for tier in crate::settings::FIXED_TIERS {
            menu.append(QVariant::from(&tier_map(tier)));
        }
        {
            let mut rust = self.as_mut().rust_mut();
            rust.play_epoch = rust.play_epoch.wrapping_add(1);
            rust.play_menu = menu;
            rust.play_title = QString::from(name);
            rust.play_id = id.to_owned();
            // 上一个请求可能还在途：它已被 epoch 作废，这里把等待标记一并清掉，
            // 否则菜单开着期间 cancel_play 会被误当成「正在取直链」而拒绝关闭
            rust.play_pending = false;
            rust.status_message = QString::default();
        }
        self.as_mut().notify();
    }

    /// 菜单里选了一项（索引对应 `FIXED_TIERS`）：这时才去请求档位和直链。
    pub fn play_selected(mut self: Pin<&mut Self>, index: i32) {
        let Ok(index) = usize::try_from(index) else {
            return;
        };
        let Some(&tier) = crate::settings::FIXED_TIERS.get(index) else {
            return;
        };
        if self.as_ref().rust().play_target().is_none() {
            self.as_mut().set_message("播放选项已过期，请重新点播放");
            return;
        }
        let name = self.play_title.to_string();
        self.as_mut().fetch_and_play(tier, &name);
    }

    /// 记下这次播放的目标（id + 文件名），并作废之前的在途请求。
    fn prepare_play(mut self: Pin<&mut Self>, id: &str, name: &str) {
        let mut rust = self.as_mut().rust_mut();
        rust.play_epoch = rust.play_epoch.wrapping_add(1);
        rust.play_menu = QVariantList::default();
        rust.play_title = QString::from(name);
        rust.play_id = id.to_owned();
        rust.play_pending = true;
    }

    /// 发请求取档位，回来后按用户要的那一档开播。
    fn fetch_and_play(mut self: Pin<&mut Self>, tier: Tier, name: &str) {
        // 新的播放取消上一次「还在加载」的播放：已在播放的 mpv 不受影响
        // （提示条消失后那条会话已从登记表里移出，这里置标志不会波及它）
        self.as_mut().cancel_pending_play();
        // 菜单已经选完，先收起（这里不清 play_id，回包回来还要用它）
        let cancel = Arc::new(Cancel::default());
        {
            let mut rust = self.as_mut().rust_mut();
            rust.play_menu = QVariantList::default();
            rust.play_pending = true;
            rust.play_loading = true;
            rust.play_loading_id = QString::from(rust.play_id.as_str());
            rust.play_loading_name = QString::from(name);
            rust.play_error = QString::default();
            rust.play_notice = QString::default();
            rust.play_cancel = Some(cancel.clone());
            // 加载态走提示条：清掉旧提醒，底栏回到页面上下文。
            rust.status_message = QString::default();
        }
        self.as_mut().notify();

        let Some(id) = self.as_ref().rust().play_target() else {
            self.as_mut()
                .fail_play_loading("播放选项已过期，请重新点播放");
            return;
        };
        // 列表里的审核标记只作为详情没下发 audit 时的兜底：
        // 播放列表刷新导致条目暂时找不到时按 false 处理，退回详情判断。
        let audit_flagged = self
            .as_ref()
            .rust()
            .find_entry(&id)
            .is_some_and(|entry| entry.audit_flagged);
        // 音乐不写官方播放历史：只让视频的样本进入上报队列
        let report_history = super::play_history::reports_progress(name);
        let name = name.to_owned();
        let session = self.backend.session.clone();
        let playback_id = id.clone();
        let epoch = self.as_ref().rust().play_epoch;
        // 开关关闭 / 非 mpv（含 VLC）：本次不准备字幕（计划取出来丢掉，别留给下一次）
        let settings = self.settings.clone();
        let (config, _) = PlayerConfig::resolve_playable(
            settings.player_backend,
            settings.player_command_override(),
            settings.player_args_override(),
        );
        // 内嵌路径不依赖系统 mpv；只有它不可用（没有 Wayland）时才回退外部，
        // 那时才需要先确认外部播放器存在，比取完直链再 spawn 失败省一次网络请求。
        let embedded_ok = wants_embedded(&settings) && crate::player::embed::available().is_ok();
        if !embedded_ok {
            if let Err(message) = config.check_available() {
                self.as_mut().fail_play_loading(&message);
                return;
            }
        }
        let wants_history = embedded_ok || config.supports_history();
        let plan = {
            let mut rust = self.as_mut().rust_mut();
            std::mem::take(&mut rust.play_subtitle_plan)
        };
        let plan = if settings.auto_subtitles && (embedded_ok || config.is_mpv()) {
            plan
        } else {
            SubtitlePlan::None
        };
        let player_label = if embedded_ok {
            "内嵌播放器".to_owned()
        } else {
            format!("播放器「{}」", config.program())
        };
        eprintln!(
            "[kichi] 字幕：开关 {}，{} → {}",
            settings.auto_subtitles,
            player_label,
            plan.summary()
        );
        // 首选字幕语言只作用于内嵌播放器：外部播放器有自己的语言配置，
        // 这里连字幕排序都不动，保持原有中文优先。
        let subtitle_languages = if embedded_ok {
            settings.player_sub_lang.tags().names
        } else {
            Vec::new()
        };

        run_in_background(
            {
                let cancel = cancel.clone();
                move || {
                    super::play_prepare::run_work(
                        session,
                        id,
                        cancel,
                        audit_flagged,
                        plan,
                        subtitle_languages,
                        wants_history,
                    )
                }
            },
            move |mut qichi, result| {
                if qichi.as_ref().rust().play_epoch != epoch {
                    return;
                }
                // 取消导致的「下载已取消」不显示：静默收尾即可
                if cancel.is_cancelled() {
                    return;
                }
                match result {
                    Ok(work) => qichi.as_mut().start_tier(
                        work,
                        tier,
                        &playback_id,
                        &name,
                        epoch,
                        report_history,
                    ),
                    Err(err) => match err {
                        // 登录失效 / 人机验证 / 限速必须全局可见：走统一出口，
                        // 但如果因此切走了登录页，提示条不该继续挂着
                        far @ (crate::session::Failure::Auth(_)
                        | crate::session::Failure::Captcha { .. }
                        | crate::session::Failure::RateLimited { .. }) => {
                            qichi.as_mut().hide_play_loading();
                            qichi.as_mut().finish_play();
                            qichi.as_mut().on_failure(far);
                        }
                        other => {
                            let text = other.to_string();
                            qichi.as_mut().fail_play_loading(&text);
                        }
                    },
                }
            },
        );
    }

    /// 拿到档位后按用户点的档位挑一个开始播放（没有就回退，见
    /// `kichi_core::types::resolve`）。
    fn start_tier(
        mut self: Pin<&mut Self>,
        work: super::play_prepare::PlayWork,
        tier: Tier,
        id: &str,
        name: &str,
        epoch: u64,
        report_history: bool,
    ) {
        let all = &work.options;
        let crate::session::subtitles::SubtitlePrep {
            assets: subtitles,
            note: subtitle_note,
            late,
        } = work.subtitles;
        if all.is_empty() {
            self.as_mut()
                .fail_play_loading(&format!("「{name}」没有可播放的链接"));
            return;
        }
        let Some((option, note)) = kichi_core::types::resolve(all, tier) else {
            let reasons = all
                .iter()
                .map(|option| format!("{}：{}", option.label, reason_text(option.reason)))
                .collect::<Vec<_>>()
                .join("；");
            self.as_mut()
                .fail_play_loading(&format!("「{name}」暂时不能播放（{reasons}）"));
            return;
        };
        let title = player::title_for(&option, name);
        // 回退过就在提示条里说清楚实际播的是哪一档，别让用户以为自己看到的是那档
        // （ADR 0003：不静默替换；说明走提示条，底栏保持页面上下文）
        let notice = super::play_prepare::combine_notes(
            note.map(|note| format!("「{name}」没有{note}，已播放{}", option.label)),
            subtitle_note,
        );
        let notice = super::play_prepare::combine_notes(notice, work.history_note);
        self.as_mut().start_player(
            &option,
            id,
            &title,
            epoch,
            notice,
            subtitles,
            late,
            work.resume,
            report_history,
        );
    }

    /// 启动外部播放器（URL 过期时先重取一次）。
    ///
    /// 成功后**保持** `playLoading`，等 `player::start` 回投「窗口已出现」。
    // 参数共同描述一次播放启动，拆结构只会让调用点来回打包。
    #[allow(clippy::too_many_arguments)]
    fn start_player(
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
            settings.player_backend,
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
                                // Ready 之后才点亮浮层：取链 / 打开阶段仍由提示条兜底
                                rust.embed_active = false;
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
            }
            StartEvent::Stopped { progress } => {
                if let Some(progress) = progress.filter(|_| report_history) {
                    self.as_mut().queue_play_progress(id, epoch, progress);
                }
                self.as_mut().clear_embed_state(epoch);
                if self.as_mut().update_active_play_session(epoch, id, false) {
                    self.as_mut().notify();
                }
            }
            StartEvent::Failed(reason) => {
                self.as_mut().clear_embed_state(epoch);
                self.as_mut().update_active_play_session(epoch, id, false);
                // `reason` 已经带了播放器上下文（…未能播放「title」：…），直接显示
                self.as_mut().fail_play_loading(&reason);
            }
            StartEvent::Cancelled => {
                self.as_mut().clear_embed_state(epoch);
                self.as_mut().update_active_play_session(epoch, id, false);
                // 取消后不给任何文字反馈
                self.as_mut().hide_play_loading();
                self.finish_play();
            }
        }
    }

    /// 提示条上的「取消」：中止取链、关掉还没出窗口的 mpv，静默收尾。
    pub fn cancel_play_loading(mut self: Pin<&mut Self>) {
        self.as_mut().cancel_pending_play();
        {
            let mut rust = self.as_mut().rust_mut();
            rust.play_loading = false;
            rust.play_loading_id = QString::default();
            rust.play_loading_name = QString::default();
            rust.play_error = QString::default();
            rust.play_notice = QString::default();
            // 不回写 status_message：底栏内容不归播放管
        }
        self.as_mut().finish_play();
    }

    /// 作废「还在加载」的那次播放（取链 + 等窗口），不动状态属性。
    ///
    /// 取链线程靠 `Cancel` 提前返回；等窗口的 mpv 靠监控线程收到取消后 `quit`。
    fn cancel_pending_play(mut self: Pin<&mut Self>) {
        let mut rust = self.as_mut().rust_mut();
        if let Some(cancel) = rust.play_cancel.take() {
            cancel.cancel();
        }
        rust.play_epoch = rust.play_epoch.wrapping_add(1);
    }

    /// 提示条上的「关闭」（错误态）：只清掉错误，不动别的。
    pub fn dismiss_play_error(mut self: Pin<&mut Self>) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.play_error = QString::default();
        }
        self.as_mut().notify();
    }

    /// 提示条自己收起回退提示时调用：清掉文案，下次回退才能再弹一次。
    pub fn dismiss_play_notice(mut self: Pin<&mut Self>) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.play_notice = QString::default();
        }
        self.as_mut().notify();
    }

    /// 真退出 kichi：连同正在播放的 mpv / 内嵌画面一起关掉（缩托盘不调）。
    pub fn shutdown_players(&self) {
        player::shutdown_all();
        crate::player::embed::shutdown_all();
    }

    /// 改走外部播放器前彻底收掉内嵌：等 mpv 线程收尾，它才会恢复
    /// `WAYLAND_SOCKET`；否则紧接着 spawn 的外部播放器会继承这个 fd，
    /// 连到内嵌代理而不是桌面合成器。浮层状态一并清掉。
    fn stop_embed_and_clear(mut self: Pin<&mut Self>) {
        if !crate::player::embed::stop_and_wait(std::time::Duration::from_millis(1200)) {
            eprintln!("[kichi] 内嵌播放器仍没收尾，外部播放器可能继承 WAYLAND_SOCKET");
        }
        let mut rust = self.as_mut().rust_mut();
        rust.embed_active = false;
        rust.embed_title = QString::default();
        rust.embed_pending_epoch = None;
    }

    /// 只收起属于这次会话的内嵌浮层；新会话已经接管时不动它。
    fn clear_embed_state(mut self: Pin<&mut Self>, epoch: u64) {
        let mut rust = self.as_mut().rust_mut();
        if rust.embed_pending_epoch != Some(epoch) {
            return;
        }
        rust.embed_pending_epoch = None;
        rust.embed_active = false;
        rust.embed_title = QString::default();
    }

    /// 内嵌浮层：暂停 / 继续
    pub fn embed_toggle_pause(&self) {
        crate::player::embed::toggle_pause();
    }

    /// 内嵌浮层：跳到某个秒数
    pub fn embed_seek(&self, seconds: f64) {
        crate::player::embed::seek_absolute(seconds);
    }

    /// 内嵌浮层：调音量（0–100）。只改内存并立刻作用到 mpv；
    /// 拖动 / 连按结束后由界面调 `embedCommitAudio` 统一落盘，避免刷盘。
    pub fn embed_set_volume(mut self: Pin<&mut Self>, volume: i32) {
        let volume = volume.clamp(0, 100);
        crate::player::embed::set_volume(volume);
        if self.embed_volume == volume {
            return;
        }
        {
            let mut rust = self.as_mut().rust_mut();
            rust.embed_volume = volume;
        }
        self.as_mut().notify();
    }

    /// 内嵌浮层：静音 / 取消静音（同样等 `embedCommitAudio` 落盘）。
    pub fn embed_toggle_mute(mut self: Pin<&mut Self>) {
        let muted = !self.embed_muted;
        crate::player::embed::set_muted(muted);
        {
            let mut rust = self.as_mut().rust_mut();
            rust.embed_muted = muted;
        }
        self.as_mut().notify();
    }

    /// 把当前音量 / 静音写进设置（界面在停止调节 0.5 秒后调用）。
    pub fn embed_commit_audio(mut self: Pin<&mut Self>) {
        let (volume, muted) = (self.embed_volume, self.embed_muted);
        if self.settings.player_volume == volume && self.settings.player_muted == muted {
            return;
        }
        self.as_mut().update_settings(|settings| {
            settings.player_volume = volume;
            settings.player_muted = muted;
        });
    }

    /// 内嵌浮层：关闭这次播放（会话收尾后照常回投 Stopped）
    pub fn embed_stop(&self) {
        crate::player::embed::stop();
    }

    /// 当前播放位置（秒）
    pub fn embed_position(&self) -> f64 {
        crate::player::embed::position().unwrap_or(0.0)
    }

    /// 片长（秒）
    pub fn embed_duration(&self) -> f64 {
        crate::player::embed::duration().unwrap_or(0.0)
    }

    /// 是否处于暂停
    pub fn embed_paused(&self) -> bool {
        crate::player::embed::is_paused()
    }

    /// 当前可切换的轨道（音轨 + 字幕轨）；菜单打开时现取，保证勾选状态最新。
    pub fn embed_tracks(&self) -> QVariantList {
        let mut list = QVariantList::default();
        for track in crate::player::embed::tracks() {
            let mut map = QVariantMap::default();
            map.insert_clone(
                &QString::from("id"),
                &QVariant::from(&i32::try_from(track.id).unwrap_or(0)),
            );
            map.insert_clone(
                &QString::from("kind"),
                &QVariant::from(&QString::from(if track.audio { "audio" } else { "sub" })),
            );
            map.insert_clone(
                &QString::from("label"),
                &QVariant::from(&QString::from(track.label.as_str())),
            );
            map.insert_clone(&QString::from("selected"), &QVariant::from(&track.selected));
            list.append(QVariant::from(&map));
        }
        list
    }

    /// 切换音轨 / 字幕轨；`id < 0` 表示关闭字幕。
    pub fn embed_select_track(&self, kind: QString, id: i32) {
        crate::player::embed::select_track(kind.to_string() == "audio", i64::from(id));
    }

    /// 内嵌会话当前音量（mpv 原生键位改过也读得到）。
    pub fn embed_current_volume(&self) -> i32 {
        crate::player::embed::volume()
    }

    /// 内嵌会话当前是否静音。
    pub fn embed_current_muted(&self) -> bool {
        crate::player::embed::muted()
    }

    /// 把 mpv 原生键位改过的音量 / 静音同步到界面属性；落盘仍走 `embedCommitAudio`。
    pub fn embed_sync_audio(mut self: Pin<&mut Self>, volume: i32, muted: bool) {
        let volume = volume.clamp(0, 100);
        if self.embed_volume == volume && self.embed_muted == muted {
            return;
        }
        {
            let mut rust = self.as_mut().rust_mut();
            rust.embed_volume = volume;
            rust.embed_muted = muted;
        }
        self.as_mut().notify();
    }

    /// 当前倍速（1.0 = 原速）；没有会话时返回 1.0。
    pub fn embed_speed(&self) -> f64 {
        crate::player::embed::speed()
    }

    /// 改倍速（50–300%）并写进设置，下次开播继续沿用。
    pub fn embed_set_speed(mut self: Pin<&mut Self>, percent: i32) {
        let percent = percent.clamp(
            i32::from(crate::settings::PLAYER_SPEED_MIN),
            i32::from(crate::settings::PLAYER_SPEED_MAX),
        );
        crate::player::embed::set_speed(f64::from(percent) / 100.0);
        let percent = u16::try_from(percent).unwrap_or(100);
        if self.settings.player_speed_percent == percent {
            return;
        }
        self.as_mut().update_settings(|settings| {
            settings.player_speed_percent = percent;
        });
    }

    /// 拖动倍速滑块时的实时预览：只改会话 / mpv，不写设置。
    pub fn embed_preview_speed(&self, percent: i32) {
        let percent = percent.clamp(
            i32::from(crate::settings::PLAYER_SPEED_MIN),
            i32::from(crate::settings::PLAYER_SPEED_MAX),
        );
        crate::player::embed::set_speed(f64::from(percent) / 100.0);
    }

    /// 把播放窗口收到的按键交给 mpv（mpv 原生键位）。
    pub fn embed_keypress(&self, key: QString) {
        crate::player::embed::keypress(&key.to_string());
    }

    /// 只收起加载态（错误 / 取消的分支共用），不动底栏。
    fn hide_play_loading(mut self: Pin<&mut Self>) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.play_loading = false;
            rust.play_loading_id = QString::default();
            rust.play_loading_name = QString::default();
            rust.play_cancel = None;
        }
        self.as_mut().notify();
    }

    /// 加载失败：收起加载态，错误进提示条，不碰底栏。
    fn fail_play_loading(mut self: Pin<&mut Self>, text: &str) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.play_loading = false;
            rust.play_loading_id = QString::default();
            rust.play_loading_name = QString::default();
            rust.play_cancel = None;
            rust.play_notice = QString::default();
            rust.play_error = QString::from(text);
        }
        self.as_mut().finish_play();
    }

    /// 用户关掉菜单：丢掉文件与状态，什么都不请求。
    ///
    /// 选中某一项后菜单会自动关闭，那次关闭**不能**取消刚发起的请求
    /// （`play_pending` 为真时直接忽略）。加载中的播放由提示条自己的
    /// [`cancel_play_loading`](Self::cancel_play_loading) 取消，这里不碰它。
    pub fn cancel_play(mut self: Pin<&mut Self>) {
        if self.play_pending {
            return;
        }
        if self.play_menu.is_empty() && self.play_title.is_empty() {
            return;
        }
        self.as_mut().clear_play_menu();
    }

    /// 本轮播放结束（成功或其他分支都算）：清掉目标，允许下一次播放。
    fn finish_play(mut self: Pin<&mut Self>) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.play_pending = false;
            rust.play_title = QString::default();
            rust.play_id.clear();
            rust.play_menu = QVariantList::default();
            rust.play_subtitle_plan = SubtitlePlan::None;
        }
        self.as_mut().notify();
    }

    /// 更新某次播放的活跃状态，并重算 QML 用的去重 id 列表。
    ///
    /// 同一文件可以被打开不止一次，所以 QML 列表按 id 去重、内部表按请求代数
    /// 分开记；只有最后一条同 id 会话退出时才不再显示「播放中」。
    fn update_active_play_session(
        mut self: Pin<&mut Self>,
        epoch: u64,
        id: &str,
        active: bool,
    ) -> bool {
        let mut rust = self.as_mut().rust_mut();
        let changed = if active {
            rust.active_play_sessions
                .insert(epoch, id.to_owned())
                .is_none_or(|previous| previous != id)
        } else {
            rust.active_play_sessions.remove(&epoch).is_some()
        };
        if !changed {
            return false;
        }
        let unique = rust
            .active_play_sessions
            .values()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        let mut ids = QStringList::default();
        for file_id in unique {
            ids.append(QString::from(file_id.as_str()));
        }
        rust.playing_ids = ids;
        true
    }

    /// 收起菜单并丢掉状态。
    fn clear_play_menu(mut self: Pin<&mut Self>) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.play_menu = QVariantList::default();
            rust.play_title = QString::default();
            rust.play_id.clear();
            rust.play_pending = false;
            rust.play_subtitle_plan = SubtitlePlan::None;
        }
        self.as_mut().notify();
    }

    /// 退出登录时清干净（连同在途请求作废）。外部播放器不动；
    /// 内嵌画面在窗口里，登出后必须一起收起。
    pub(crate) fn reset_play(mut self: Pin<&mut Self>) {
        crate::player::embed::stop();
        {
            let mut rust = self.as_mut().rust_mut();
            if let Some(cancel) = rust.play_cancel.take() {
                cancel.cancel();
            }
            let reset_epoch = rust.play_epoch.wrapping_add(1);
            rust.play_epoch = reset_epoch;
            rust.play_progress.reset(reset_epoch);
            rust.play_menu = QVariantList::default();
            rust.play_title = QString::default();
            rust.play_id.clear();
            rust.play_pending = false;
            rust.play_loading = false;
            rust.play_loading_id = QString::default();
            rust.play_loading_name = QString::default();
            rust.active_play_sessions.clear();
            rust.playing_ids = QStringList::default();
            rust.play_error = QString::default();
            rust.play_notice = QString::default();
            rust.play_subtitle_plan = SubtitlePlan::None;
            rust.embed_active = false;
            rust.embed_title = QString::default();
            rust.embed_pending_epoch = None;
        }
    }
}

/// 设置里的偏好 → 直接要播的固定档位（`Ask` 返回 None = 弹菜单）。
fn fixed_tier_of(quality: crate::settings::Quality) -> Option<Tier> {
    match quality {
        crate::settings::Quality::Ask => None,
        crate::settings::Quality::Original => Some(Tier::Original),
        crate::settings::Quality::Transcode(name) => Some(Tier::Transcode(name)),
    }
}

/// 固定档位 → QML 小 map。
fn tier_map(tier: Tier) -> QVariantMap {
    let text = |value: &str| QVariant::from(&QString::from(value));
    let mut map = QVariantMap::default();
    map.insert_clone(&QString::from("label"), &text(tier.label()));
    map.insert_clone(
        &QString::from("original"),
        &QVariant::from(&(tier == Tier::Original)),
    );
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reasons_become_readable_chinese() {
        assert_eq!(
            reason_text(Some(kichi_core::media::REASON_QUOTA)),
            "需要额外播放配额"
        );
        assert_eq!(reason_text(None), "");
        assert_eq!(reason_text(Some("other")), "other");
    }

    #[test]
    fn menu_is_the_four_fixed_tiers_without_any_request() {
        let labels: Vec<_> = crate::settings::FIXED_TIERS
            .iter()
            .map(|tier| tier.label())
            .collect();
        assert_eq!(labels, vec!["原画", "1080P", "720P", "480P"]);
    }

    #[test]
    fn menu_map_never_carries_a_url() {
        let map = tier_map(Tier::Transcode("1080P"));
        let debug = format!("{map:?}");
        assert!(!debug.contains("cdn.invalid"));
        assert!(!debug.contains("sign="));
        assert_eq!(map.len(), 2);
    }
}
