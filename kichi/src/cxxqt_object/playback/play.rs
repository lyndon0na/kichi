//! 播放入口：档位菜单、开始播放与取消。
//!
//! 菜单**不等网络**：四项（原画 / 1080P / 720P / 480P）是常量，点「播放」立刻弹出；
//! 用户在菜单里选了某一档，才发那一次「取直链」的请求（签名链接只能从服务端拿）。
//!
//! 直链只活在 Rust 侧的局部变量里，QML 拿到的菜单里**没有 URL**，
//! 也就没有把签名链接写进界面日志或剪贴板的机会。
//!
//! 取链在 [`super::play_fetch`]，播放器启动与事件在 [`super::play_start`]，
//! 内嵌浮层接口在 [`super::play_embed`]，加载态与收尾在 [`super::play_state`]。

use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QMap, QMapPair_QString_QVariant, QString, QVariant};
use kichi_core::types::Tier;

use super::qobject;
use super::QVariantList;
use crate::settings::{PlayerBackend, Settings};

type QVariantMap = QMap<QMapPair_QString_QVariant>;

/// 设置要求内嵌播放，且没有用 `KICHI_PLAYER` 临时改走外部命令。
///
/// Flatpak 沙箱里只有内嵌播放器：设置页不显示外部选项，`KICHI_PLAYER`
pub(crate) fn wants_embedded(settings: &Settings) -> bool {
    let sandboxed = crate::platform::flatpak_sandbox();
    settings.effective_player_backend() == PlayerBackend::Embedded
        && (sandboxed || std::env::var_os("KICHI_PLAYER").is_none())
}

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
        let Some(id) = self.as_ref().rust().play_target() else {
            self.as_mut().set_message("播放选项已过期，请重新点播放");
            return;
        };
        let name = self.play_title.to_string();
        // 菜单可能开了一会儿：期间目录刷新 / 连播切集都可能改过快照，重新算一次。
        let plan = {
            let this = self.as_ref();
            let rust = this.rust();
            super::play_prepare::plan_for(&id, &rust.entries, &rust.search_index)
        };
        self.as_mut().rust_mut().play_subtitle_plan = plan;
        // 菜单选完才算真正开播：这时才快照连播队列，关掉菜单不会动当前队列。
        self.as_mut().begin_play_queue(&id, &name);
        self.as_mut().fetch_and_play(tier, &name);
    }

    /// 记下这次播放的目标（id + 文件名），并作废之前的在途请求。
    fn prepare_play(mut self: Pin<&mut Self>, id: &str, name: &str) {
        // 连播队列只在这里建一次快照：菜单开着时目录刷新不影响本次播放。
        self.as_mut().begin_play_queue(id, name);
        self.as_mut().prepare_play_state(id, name);
    }

    /// 只重置播放目标状态，不动连播队列（队列内切集走这里）。
    pub(crate) fn prepare_play_state(mut self: Pin<&mut Self>, id: &str, name: &str) {
        let mut rust = self.as_mut().rust_mut();
        rust.play_epoch = rust.play_epoch.wrapping_add(1);
        rust.play_menu = QVariantList::default();
        rust.play_title = QString::from(name);
        rust.play_id = id.to_owned();
        rust.play_pending = true;
    }
}

/// 设置里的偏好 → 直接要播的固定档位（`Ask` 返回 None = 弹菜单）。
pub(super) fn fixed_tier_of(quality: crate::settings::Quality) -> Option<Tier> {
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
