use super::super::types::{is_language_code, tags_for_locale, LanguageTags};
use super::super::*;
use super::*;

#[test]
fn enum_parsing_is_lenient() {
    assert_eq!(Quality::parse("original"), Quality::Original);
    assert_eq!(
        Quality::parse("transcode:1080P"),
        Quality::Transcode("1080P")
    );
    assert_eq!(
        Quality::parse(" transcode:720p "),
        Quality::Transcode("720P"),
        "档位名大小写不该影响识别"
    );
    assert_eq!(
        Quality::parse("transcode"),
        Quality::Ask,
        "旧版的「优先最高转码档」没有对应档位，回每次询问而不是替用户猜"
    );
    assert_eq!(Quality::parse("transcode:1440P"), Quality::Ask);
    assert_eq!(Quality::parse("nonsense"), Quality::Ask);
    assert_eq!(SortField::parse("size"), SortField::Size);
    assert_eq!(SortField::parse(""), SortField::Name);
    assert_eq!(ContentViewMode::parse("grid"), ContentViewMode::Grid);
    assert_eq!(ContentViewMode::parse(" GRID "), ContentViewMode::Grid);
    assert_eq!(ContentViewMode::parse("list"), ContentViewMode::List);
    assert_eq!(ContentViewMode::parse("nonsense"), ContentViewMode::List);
    assert_eq!(PlayerBackend::parse("vlc"), PlayerBackend::Vlc);
    assert_eq!(PlayerBackend::parse(" CUSTOM "), PlayerBackend::Custom);
    assert_eq!(PlayerBackend::parse("nonsense"), PlayerBackend::Mpv);
    assert_eq!(
        PreferredLanguage::parse(" JA "),
        PreferredLanguage::Japanese
    );
    assert_eq!(
        PreferredLanguage::parse("nonsense"),
        PreferredLanguage::Auto
    );
    for language in [
        PreferredLanguage::Auto,
        PreferredLanguage::Chinese,
        PreferredLanguage::Cantonese,
        PreferredLanguage::English,
        PreferredLanguage::Japanese,
        PreferredLanguage::Korean,
    ] {
        assert_eq!(
            PreferredLanguage::parse(language.as_str()),
            language,
            "{language:?}"
        );
    }
}

#[test]
fn flatpak_uses_embedded_without_rewriting_the_saved_choice() {
    let settings = Settings {
        player_backend: PlayerBackend::Vlc,
        ..Settings::default()
    };

    assert_eq!(
        settings.effective_player_backend_in(true),
        PlayerBackend::Embedded,
        "Flatpak 沙箱里只能内嵌"
    );
    assert_eq!(
        settings.effective_player_backend_in(false),
        PlayerBackend::Vlc,
        "非 Flatpak 保持用户选择"
    );
    assert_eq!(
        settings.player_backend,
        PlayerBackend::Vlc,
        "共用配置文件不能被 Flatpak 悄悄改写"
    );
}

#[test]
fn locale_language_aliases_and_fallbacks() {
    let chinese = tags_for_locale("zh_CN.UTF-8").expect("简体中文");
    assert!(chinese.mpv.contains(&"zh".to_owned()));
    assert!(chinese.names.contains(&"简体".to_owned()));
    assert!(tags_for_locale("zh_TW")
        .expect("繁体中文")
        .names
        .contains(&"繁體".to_owned()));
    assert_eq!(
        tags_for_locale("ja_JP.UTF-8"),
        Some(PreferredLanguage::Japanese.tags())
    );
    assert_eq!(
        tags_for_locale("en_US"),
        Some(PreferredLanguage::English.tags())
    );
    assert_eq!(
        tags_for_locale("yue_HK"),
        Some(PreferredLanguage::Cantonese.tags())
    );
    assert_eq!(
        tags_for_locale("fr_FR.UTF-8").expect("其它语言"),
        LanguageTags {
            mpv: vec!["fr".to_owned()],
            names: vec!["fr".to_owned()],
        }
    );
    assert_eq!(tags_for_locale("C.UTF-8"), None);
    assert_eq!(tags_for_locale("POSIX"), None);
    assert_eq!(tags_for_locale(""), None);
}

#[test]
fn generic_locale_codes_are_two_or_three_ascii_letters() {
    assert!(is_language_code("fr"));
    assert!(is_language_code("fil"));
    assert!(!is_language_code("f"));
    assert!(!is_language_code("french"));
    assert!(!is_language_code("f1"));
}

#[test]
fn subtitle_colors_must_be_hex_and_speed_is_clamped_both_ways() {
    assert!(valid_color("#FFFFFFFF"));
    assert!(valid_color("#123456"));
    assert!(!valid_color("white"));
    assert!(!valid_color("#FFF"));
    assert!(!valid_color("#GGGGGG"));

    let low = Settings {
        player_speed_percent: 1,
        ..Settings::default()
    }
    .normalized();
    assert_eq!(low.player_speed_percent, PLAYER_SPEED_MIN);
    let high = Settings {
        player_speed_percent: 301,
        ..Settings::default()
    }
    .normalized();
    assert_eq!(high.player_speed_percent, PLAYER_SPEED_MAX);
}

#[test]
fn quality_roundtrips_through_its_string_form() {
    for quality in [
        Quality::Ask,
        Quality::Original,
        Quality::Transcode("1080P"),
        Quality::Transcode("720P"),
        Quality::Transcode("480P"),
    ] {
        assert_eq!(Quality::parse(quality.as_str()), quality, "{quality:?}");
    }
}

#[test]
fn blank_player_overrides_fall_back_to_defaults() {
    let mut settings = Settings {
        player_command: "  ".into(),
        player_args: "\n".into(),
        ..Settings::default()
    };
    assert!(settings.player_command_override().is_none());
    assert!(settings.player_args_override().is_none());
    settings.player_command = "mpv".into();
    assert_eq!(settings.player_command_override(), Some("mpv"));
}
