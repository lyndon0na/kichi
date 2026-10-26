//! 播放：取档位 → 弹菜单 → 选中后用外部 mpv 串流。
//!
//! 直链只活在 Rust 侧（`pending_options`），QML 拿到的菜单里**没有 URL**，
//! 也就没有把签名链接写进界面日志或剪贴板的机会。

use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QMap, QMapPair_QString_QVariant, QString, QVariant};
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
    /// 右键菜单点「播放」：取该文件的播放档位，填好菜单（只有一个可用档时直接播）。
    pub fn request_play(mut self: Pin<&mut Self>, id: QString) {
        let id = id.to_string();
        let Some(name): Option<String> = self.as_ref().rust().entry_name(&id) else {
            self.as_mut()
                .set_message("该文件已不在当前列表中，请刷新后重试", false);
            return;
        };
        let session = self.backend.session.clone();
        let epoch = {
            let mut rust = self.as_mut().rust_mut();
            rust.play_epoch = rust.play_epoch.wrapping_add(1);
            rust.pending_options.clear();
            rust.play_menu = QVariantList::default();
            rust.play_title = QString::from(name.as_str());
            rust.play_epoch
        };
        self.as_mut()
            .set_message(&format!("正在获取「{name}」的播放链接……"), true);

        run_in_background(
            move || media::play_options(&session, &id),
            move |mut qichi, result| {
                if qichi.as_ref().rust().play_epoch != epoch {
                    return;
                }
                match result {
                    Ok(options) => qichi.as_mut().show_play_menu(options, &name),
                    Err(err) => qichi.as_mut().on_failure(err),
                }
            },
        );
    }

    /// 菜单里选了一项（索引对应 `playMenu`）。
    pub fn play_selected(mut self: Pin<&mut Self>, index: i32) {
        if index < 0 {
            return;
        }
        let Some(option) = self
            .as_ref()
            .rust()
            .pending_options
            .get(index as usize)
            .cloned()
        else {
            self.as_mut()
                .set_message("播放选项已过期，请重新点播放", false);
            return;
        };
        if !option.playable() {
            self.as_mut().set_message(
                &format!(
                    "「{}」暂时不能播放：{}",
                    option.label,
                    reason_text(option.reason)
                ),
                false,
            );
            return;
        }
        let title = player::title_for(&option, self.play_title.to_string().as_str());
        self.as_mut().start_player(&option, &title);
        self.as_mut().clear_play_menu();
    }

    /// 后台取完档位后的收尾（只在主线程调用）。
    fn show_play_menu(mut self: Pin<&mut Self>, options: Vec<PlayOption>, name: &str) {
        let playable: Vec<usize> = options
            .iter()
            .enumerate()
            .filter(|(_, option)| option.playable())
            .map(|(index, _)| index)
            .collect();
        if options.is_empty() {
            self.as_mut()
                .set_message(&format!("「{name}」没有可播放的链接"), false);
            return;
        }
        if playable.is_empty() {
            let reasons = options
                .iter()
                .map(|option| format!("{}：{}", option.label, reason_text(option.reason)))
                .collect::<Vec<_>>()
                .join("；");
            self.as_mut()
                .set_message(&format!("「{name}」暂时不能播放（{reasons}）"), false);
            return;
        }
        // 按偏好直通：只有一个能播，或用户设了默认清晰度且该档可用
        let direct = pick_preferred(&options, self.settings.preferred_quality);
        if let Some(index) = direct {
            let option = options[index].clone();
            {
                let mut rust = self.as_mut().rust_mut();
                rust.pending_options = options;
            }
            let title = player::title_for(&option, name);
            self.as_mut().start_player(&option, &title);
            self.as_mut().clear_play_menu();
            return;
        }

        let mut menu = QVariantList::default();
        for option in &options {
            menu.append(QVariant::from(&option_map(option)));
        }
        {
            let mut rust = self.as_mut().rust_mut();
            rust.pending_options = options;
            rust.play_menu = menu;
            rust.busy = false;
            rust.status_message = QString::from(format!("「{name}」请选择清晰度").as_str());
        }
        self.as_mut().notify();
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

    /// 用户关掉菜单：丢掉档位（连同直链），不留下任何状态。
    pub fn cancel_play(mut self: Pin<&mut Self>) {
        if self.pending_options.is_empty() {
            return;
        }
        self.as_mut().clear_play_menu();
    }

    /// 收起菜单并丢掉直链。
    fn clear_play_menu(mut self: Pin<&mut Self>) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.play_menu = QVariantList::default();
            rust.pending_options.clear();
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
            rust.pending_options.clear();
        }
    }
}

/// 决定是否免菜单直通播放，返回要播的档位下标。
///
/// 语义（2026-10-26 按用户实测反馈修正）：
/// - 「每次询问」：**永远弹菜单**，哪怕只有一个档。用户选了这个就是要自己确认，
///   偷偷跳过菜单会让人觉得设置没作用。
/// - 「优先原画」：有可用原画就播。
/// - 「优先最高转码档」：先找转码档；**一个转码档都没有时用原画兜底**
///   （实测很多文件只有原画一档，此时弹菜单对用户毫无意义）。
/// - 偏好档确实无法满足（既无该档也没有可接受兜底）：返回 `None` 弹菜单解释。
fn pick_preferred(options: &[PlayOption], quality: crate::settings::Quality) -> Option<usize> {
    use crate::settings::Quality;
    let playable: Vec<usize> = options
        .iter()
        .enumerate()
        .filter(|(_, option)| option.playable())
        .map(|(index, _)| index)
        .collect();
    if playable.is_empty() {
        return None;
    }
    match quality {
        // 明确要求「每次询问」：不给捷径
        Quality::Ask => None,
        Quality::Original => playable
            .iter()
            .copied()
            .find(|index| options[*index].original),
        Quality::Transcode => {
            let best_transcode = playable
                .iter()
                .copied()
                .filter(|index| !options[*index].original)
                .max_by_key(|index| options[*index].height.unwrap_or(0));
            // 没有转码档时用原画兜底：这类文件只有原画，弹菜单没有可选项
            best_transcode.or_else(|| {
                playable
                    .iter()
                    .copied()
                    .find(|index| options[*index].original)
            })
        }
    }
}

/// 一个档位 → QML 小 map（不含 URL）。
fn option_map(option: &PlayOption) -> QVariantMap {
    let text = |value: &str| QVariant::from(&QString::from(value));
    let mut map = QVariantMap::default();
    map.insert_clone(&QString::from("label"), &text(&option.label));
    map.insert_clone(
        &QString::from("available"),
        &QVariant::from(&option.available),
    );
    map.insert_clone(
        &QString::from("original"),
        &QVariant::from(&option.original),
    );
    map.insert_clone(&QString::from("reason"), &text(&reason_text(option.reason)));
    map.insert_clone(
        &QString::from("height"),
        &QVariant::from(&option.height.map(|h| h as i32).unwrap_or(0)),
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

    fn option(label: &str, height: Option<u32>, original: bool, available: bool) -> PlayOption {
        PlayOption {
            label: label.into(),
            url: "https://cdn.invalid/x?sign=S".into(),
            kind: kichi_core::types::StreamKind::Original,
            height,
            original,
            available,
            reason: (!available).then_some(kichi_core::media::REASON_QUOTA),
        }
    }

    fn sample_menu() -> Vec<PlayOption> {
        vec![
            option("原画", None, true, true),
            option("1080P", Some(1080), false, true),
            option("720P", Some(720), false, true),
        ]
    }

    #[test]
    fn preferred_quality_picks_the_right_tier() {
        use crate::settings::Quality;
        let menu = sample_menu();
        assert_eq!(
            pick_preferred(&menu, Quality::Ask),
            None,
            "选了「每次询问」就必须弹菜单，哪怕只有一个档"
        );
        assert_eq!(pick_preferred(&menu, Quality::Original), Some(0));
        assert_eq!(
            pick_preferred(&menu, Quality::Transcode),
            Some(1),
            "转码挑最高档"
        );
    }

    #[test]
    fn ask_always_shows_the_menu_even_with_a_single_tier() {
        use crate::settings::Quality;
        // 实测大量文件只有「原画」一档：此时「每次询问」也必须弹菜单，
        // 否则用户会以为设置根本没生效
        let menu = vec![option("原画", None, true, true)];
        assert_eq!(pick_preferred(&menu, Quality::Ask), None);
    }

    #[test]
    fn transcode_preference_falls_back_to_original_when_no_transcode_exists() {
        use crate::settings::Quality;
        // 实测：很多文件只有原画档，选「优先最高转码档」时应该直接播原画，
        // 弹一个只有一项的菜单对用户毫无意义
        let menu = vec![option("原画", None, true, true)];
        assert_eq!(pick_preferred(&menu, Quality::Transcode), Some(0));
        // 原画不可用且没有转码档 -> 只能弹菜单
        let menu = vec![option("原画", None, true, false)];
        assert_eq!(pick_preferred(&menu, Quality::Transcode), None);
    }

    #[test]
    fn unavailable_preference_falls_back_to_menu() {
        use crate::settings::Quality;
        // 原画不可用、但有转码档：偏好原画时回退菜单，让用户看到原因
        let menu = vec![
            option("原画", None, true, false),
            option("1080P", Some(1080), false, true),
        ];
        assert_eq!(pick_preferred(&menu, Quality::Original), None);
        // 有可用转码档时优先转码，不受原画不可用影响
        assert_eq!(pick_preferred(&menu, Quality::Transcode), Some(1));
    }

    #[test]
    fn nothing_playable_never_returns_an_index() {
        use crate::settings::Quality;
        let menu = vec![option("原画", None, true, false)];
        for quality in [Quality::Ask, Quality::Original, Quality::Transcode] {
            assert_eq!(pick_preferred(&menu, quality), None);
        }
        assert_eq!(pick_preferred(&[], Quality::Original), None);
    }

    #[test]
    fn menu_map_never_carries_the_signed_url() {
        let option = PlayOption {
            label: "原画".into(),
            url: "https://cdn.invalid/secret?sign=SECRET".into(),
            kind: kichi_core::types::StreamKind::Original,
            height: None,
            original: true,
            available: true,
            reason: None,
        };
        let map = option_map(&option);
        let debug = format!("{map:?}");
        assert!(!debug.contains("SECRET"), "菜单里不得出现直链");
        assert_eq!(map.len(), 5);
    }
}
