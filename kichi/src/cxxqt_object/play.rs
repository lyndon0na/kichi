//! 播放：弹固定档位菜单 → 选中后取直链 → 用外部 mpv 串流。
//!
//! 菜单**不等网络**：四项（原画 / 1080P / 720P / 480P）是常量，点「播放」立刻弹出；
//! 用户在菜单里选了某一档，才发那一次「取直链」的请求（签名链接只能从服务端拿）。
//!
//! 直链只活在 Rust 侧的局部变量里，QML 拿到的菜单里**没有 URL**，
//! 也就没有把签名链接写进界面日志或剪贴板的机会。
//!
//! 等待过程中 `playLoading` + `playLoadingId` 指向当前文件，文件缩略图据此
//! 显示加载状态并提供取消；离开文件页时 `PlayToast.qml` 接替显示取消入口。
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

type QVariantMap = QMap<QMapPair_QString_QVariant>;

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
        // 新的播放取消上一次「还在加载」的那次（已在播放的 mpv 不受影响）：
        // 否则它被 epoch 作废后，旧缩略图会永远停在「正在加载…」
        self.as_mut().cancel_play_loading();
        // 在点播放这一刻快照字幕计划：菜单开着时目录刷新不影响本次播放。
        // 两条路径（直接播 / 先弹菜单）都要有，所以放在分支之前。
        let plan = {
            let this = self.as_ref();
            let rust = this.rust();
            super::play_prepare::plan_for(&id, &rust.entries, &rust.search_index)
        };
        self.as_mut().rust_mut().play_subtitle_plan = plan;
        // 设置里指定了档位：等同用户在菜单里点了那一项，不弹菜单
        if let Some(tier) = fixed_tier_of(self.settings.preferred_quality) {
            self.as_mut().prepare_play(&id, &name);
            self.as_mut().fetch_and_play(tier, &name);
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
            rust.play_title = QString::from(name.as_str());
            rust.play_id = id;
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
        let name = name.to_owned();
        let session = self.backend.session.clone();
        let playback_id = id.clone();
        let epoch = self.as_ref().rust().play_epoch;
        // 开关关闭 / 非 mpv：本次不准备字幕（计划取出来丢掉，别留给下一次）
        let settings = self.settings.clone();
        let config = PlayerConfig::resolve(
            settings.player_command_override(),
            settings.player_args_override(),
        );
        let wants_history = config.is_mpv();
        let plan = {
            let mut rust = self.as_mut().rust_mut();
            std::mem::take(&mut rust.play_subtitle_plan)
        };
        let plan = if settings.auto_subtitles && config.is_mpv() {
            plan
        } else {
            SubtitlePlan::None
        };
        eprintln!(
            "[kichi] 字幕：开关 {}，播放器「{}」→ {}",
            settings.auto_subtitles,
            config.program(),
            plan.summary()
        );

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
                    Ok(work) => qichi
                        .as_mut()
                        .start_tier(work, tier, &playback_id, &name, epoch),
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
    ) {
        if option.is_expired(None) {
            self.as_mut()
                .fail_play_loading("播放链接已过期，请重新点播放");
            return;
        }
        let settings = self.settings.clone();
        let config = PlayerConfig::resolve(
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
        let event_notice = notice;
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
            move |event| {
                let playback_id = playback_id.clone();
                let event_notice = event_notice.clone();
                enqueue_ui(move |mut qichi| {
                    qichi
                        .as_mut()
                        .on_play_event(event, event_notice, &playback_id, epoch);
                });
            },
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
                    // 成功后清掉旧提醒：不常驻「已用 … 播放…」
                    rust.status_message = QString::default();
                }
                self.finish_play();
            }
            StartEvent::Progress { progress } => {
                self.as_mut().queue_play_progress(id, epoch, progress);
            }
            StartEvent::Stopped { progress } => {
                if let Some(progress) = progress {
                    self.as_mut().queue_play_progress(id, epoch, progress);
                }
                if self.as_mut().update_active_play_session(epoch, id, false) {
                    self.as_mut().notify();
                }
            }
            StartEvent::Failed(reason) => {
                self.as_mut().update_active_play_session(epoch, id, false);
                // `reason` 已经带了上下文（mpv 未能播放「title」：…），直接显示
                self.as_mut().fail_play_loading(&reason);
            }
            StartEvent::Cancelled => {
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

    /// 真退出 kichi：连同正在播放的 mpv 一起关掉（缩托盘不调）。
    pub fn shutdown_players(&self) {
        player::shutdown_all();
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

    /// 退出登录时清干净（连同在途请求作废）。已经在播的 mpv 不动。
    pub(crate) fn reset_play(mut self: Pin<&mut Self>) {
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
