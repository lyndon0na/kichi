//! 取直链与开始播放：按档位请求、失败提示与过期重试。

use std::pin::Pin;
use std::sync::Arc;

use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;
use kichi_core::types::Tier;

use super::actions::run_in_background;
use super::play::{reason_text, wants_embedded};
use super::qobject;
use super::QVariantList;
use crate::player;
use crate::player::PlayerConfig;
use crate::session::download::Cancel;
use crate::session::subtitles::SubtitlePlan;

impl qobject::Kichi {
    /// 发请求取档位，回来后按用户要的那一档开播。
    pub(crate) fn fetch_and_play(mut self: Pin<&mut Self>, tier: Tier, name: &str) {
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
        // 队列切集：片尾预取的链接命中就省掉这一次详情请求。
        let prefetched = self.as_mut().take_prefetched(&id);
        {
            let mut rust = self.as_mut().rust_mut();
            rust.play_queue_state.current_tier = Some(tier);
        }
        // 列表里的审核标记只作为详情没下发 audit 时的兜底：
        // 播放列表刷新导致条目暂时找不到时按 false 处理，退回详情判断。
        let audit_flagged = {
            let this = self.as_ref();
            let rust = this.rust();
            rust.find_entry(&id)
                .or_else(|| {
                    rust.play_queue_state
                        .playlist
                        .entries()
                        .iter()
                        .find(|entry| entry.id == id)
                        .cloned()
                })
                .is_some_and(|entry| entry.audit_flagged)
        };
        // 音乐不写官方播放历史：只让视频的样本进入上报队列
        let report_history = super::play_history::reports_progress(name);
        let name = name.to_owned();
        let session = self.backend.session.clone();
        let playback_id = id.clone();
        let epoch = self.as_ref().rust().play_epoch;
        // 开关关闭 / 非 mpv（含 VLC）：本次不准备字幕（计划取出来丢掉，别留给下一次）
        let settings = self.settings.clone();
        let (config, _) = PlayerConfig::resolve_playable(
            settings.effective_player_backend(),
            settings.player_command_override(),
            settings.player_args_override(),
        );
        // 内嵌路径不依赖系统 mpv；只有它不可用（没有 Wayland）时才回退外部，
        // 那时才需要先确认外部播放器存在，比取完直链再 spawn 失败省一次网络请求。
        // Flatpak 沙箱里没有外部播放器可回退，直接给出明确原因。
        let embedded_ok = wants_embedded(&settings) && crate::player::embed::available().is_ok();
        if !embedded_ok {
            if crate::platform::flatpak_sandbox() {
                let reason = crate::player::embed::available()
                    .err()
                    .unwrap_or_else(|| "内嵌播放器不可用".to_owned());
                self.as_mut()
                    .fail_play_loading(&format!("Flatpak 版只提供内嵌播放器：{reason}"));
                return;
            }
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
                        prefetched,
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
}
