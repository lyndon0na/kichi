//! 播放：弹固定档位菜单 → 选中后取直链 → 用外部 mpv 串流。
//!
//! 菜单**不等网络**：四项（原画 / 1080P / 720P / 480P）是常量，点「播放」立刻弹出；
//! 用户在菜单里选了某一档，才发那一次「取直链」的请求（签名链接只能从服务端拿）。
//!
//! 直链只活在 Rust 侧的局部变量里，QML 拿到的菜单里**没有 URL**，
//! 也就没有把签名链接写进界面日志或剪贴板的机会。

use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QMap, QMapPair_QString_QVariant, QString, QVariant};
use kichi_core::types::Tier;
use kichi_core::PlayOption;

use super::actions::run_in_background;
use super::qobject;
use super::QVariantList;
use crate::player::{self, mask_url, PlayerConfig};
use crate::session::media;

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
                .set_message("该文件已不在当前列表中，请刷新后重试", false);
            return;
        };
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
            rust.busy = false;
            rust.status_message = QString::from(format!("「{name}」请选择清晰度").as_str());
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
            self.as_mut()
                .set_message("播放选项已过期，请重新点播放", false);
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
        // 菜单已经选完，先收起（这里不清 play_id，回包回来还要用它）
        {
            let mut rust = self.as_mut().rust_mut();
            rust.play_menu = QVariantList::default();
            rust.play_pending = true;
        }
        let Some(id) = self.as_ref().rust().play_target() else {
            self.as_mut()
                .set_message("播放选项已过期，请重新点播放", false);
            return;
        };
        let name = name.to_owned();
        let session = self.backend.session.clone();
        let epoch = self.as_ref().rust().play_epoch;
        self.as_mut()
            .set_message(&format!("正在获取「{name}」的播放链接……"), true);

        run_in_background(
            move || media::play_options(&session, &id),
            move |mut qichi, result| {
                if qichi.as_ref().rust().play_epoch != epoch {
                    return;
                }
                match result {
                    Ok(options) => {
                        qichi.as_mut().start_tier(&options, tier, &name);
                    }
                    Err(err) => qichi.as_mut().on_failure(err),
                }
            },
        );
    }

    /// 拿到档位后按用户点的档位挑一个开始播放（没有就回退，见 [`resolve`]）。
    fn start_tier(mut self: Pin<&mut Self>, all: &[PlayOption], tier: Tier, name: &str) {
        if all.is_empty() {
            self.as_mut()
                .set_message(&format!("「{name}」没有可播放的链接"), false);
            self.as_mut().finish_play();
            return;
        }
        let Some((option, note)) = kichi_core::types::resolve(all, tier) else {
            let reasons = all
                .iter()
                .map(|option| format!("{}：{}", option.label, reason_text(option.reason)))
                .collect::<Vec<_>>()
                .join("；");
            self.as_mut()
                .set_message(&format!("「{name}」暂时不能播放（{reasons}）"), false);
            self.as_mut().finish_play();
            return;
        };
        let title = player::title_for(&option, name);
        self.as_mut().start_player(&option, &title);
        self.as_mut().finish_play();
        // 回退过就说清楚实际播的是哪一档，别让用户以为自己看到的是选的那档
        // （放在 finish_play 之后：它只清播放目标，不动状态栏）
        if let Some(note) = note {
            self.as_mut().set_message(
                &format!("「{name}」没有{note}，已播放{}", option.label),
                false,
            );
        }
    }

    /// 启动外部播放器（URL 过期时先重取一次）。
    fn start_player(mut self: Pin<&mut Self>, option: &PlayOption, title: &str) {
        if option.is_expired(None) {
            self.as_mut()
                .set_message("播放链接已过期，请重新点播放", false);
            return;
        }
        let settings = self.settings.clone();
        let config = PlayerConfig::resolve(
            settings.player_command_override(),
            settings.player_args_override(),
        );
        match player::launch(&config, &option.url, title) {
            Ok(()) => self.as_mut().set_message(
                &format!(
                    "已用 {} 播放「{title}」（{}）",
                    config.program(),
                    option.label
                ),
                false,
            ),
            Err(err) => self.as_mut().set_message(&err, false),
        }
        eprintln!(
            "[kichi] 播放「{title}」{} → {}",
            option.label,
            mask_url(&option.url)
        );
    }

    /// 用户关掉菜单：丢掉文件与状态，什么都不请求。
    ///
    /// 选中某一项后菜单会自动关闭，那次关闭**不能**取消刚发起的请求
    /// （`play_pending` 为真时直接忽略）。
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
        }
        self.as_mut().notify();
    }

    /// 收起菜单并丢掉状态。
    fn clear_play_menu(mut self: Pin<&mut Self>) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.play_menu = QVariantList::default();
            rust.play_title = QString::default();
            rust.play_id.clear();
            rust.play_pending = false;
        }
        self.as_mut().notify();
    }

    /// 退出登录时清干净（连同在途请求作废）。
    pub(crate) fn reset_play(mut self: Pin<&mut Self>) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.play_epoch = rust.play_epoch.wrapping_add(1);
            rust.play_menu = QVariantList::default();
            rust.play_title = QString::default();
            rust.play_id.clear();
            rust.play_pending = false;
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
