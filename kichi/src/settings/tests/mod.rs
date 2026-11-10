//! 设置读写、迁移与路径测试。

use std::path::PathBuf;

use super::*;

mod types;

fn temp_path(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kichi-settings-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    dir.join(name)
}

#[test]
fn real_config_file_is_read_correctly() {
    let path = crate::settings::default_path();
    eprintln!("读取路径: {}", path.display());
    let (s, w) = load(&path);
    eprintln!("close_to_tray = {}, warning = {w:?}", s.close_to_tray);
    if path.exists() {
        eprintln!(
            "文件内容: {}",
            std::fs::read_to_string(&path).unwrap_or_default()
        );
    }
}

#[test]
fn missing_file_means_defaults_without_warning() {
    let path = temp_path("absent.json");
    let _ = std::fs::remove_file(&path);
    let (settings, warning) = load(&path);
    assert_eq!(settings, Settings::default());
    assert!(settings.player_mpv_conf, "embed-mpv.conf 默认开启");
    assert!(settings.player_input_conf, "embed-input.conf 默认开启");
    assert!(settings.player_vulkan, "内嵌播放默认使用 Vulkan 渲染");
    assert!(warning.is_none(), "文件不存在不算错误");
}

#[test]
fn roundtrip_keeps_every_field() {
    let path = temp_path("roundtrip.json");
    let settings = Settings {
        preferred_quality: Quality::Transcode("1080P"),
        player_backend: PlayerBackend::Custom,
        player_command: "mpv".into(),
        player_args: "--fs --vo=gpu".into(),
        notify_enabled: false,
        auto_play_next: false,
        player_audio_lang: PreferredLanguage::Japanese,
        player_sub_lang: PreferredLanguage::Chinese,
        player_mpv_conf: false,
        player_input_conf: false,
        player_volume: 65,
        player_muted: true,
        player_speed_percent: 125,
        player_sub_font_size: 44,
        player_sub_pos: 90,
        player_sub_color: "#FFFFFF00".into(),
        player_sub_outline_size: 3,
        player_sub_outline_color: "#FF3366CC".into(),
        close_to_tray: true,
        content_view: ContentViewMode::Grid,
        sort_field: SortField::Modified,
        sort_ascending: false,
        window_width: 1280,
        window_height: 720,
        window_maximized: true,
        download_parallel: 5,
        download_connections: 6,
        download_rate_mib: 12,
        upload_parallel: 4,
        upload_rate_mib: 8,
        auto_resume_downloads: false,
        auto_resume_uploads: true,
        local_index_enabled: true,
        ..Settings::default()
    };
    save(&path, &settings).expect("保存失败");
    let (loaded, warning) = load(&path);
    assert!(warning.is_none());
    assert_eq!(loaded, settings);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn saved_file_is_private() {
    let path = temp_path("private.json");
    let _ = std::fs::remove_file(&path);
    save(&path, &Settings::default()).expect("保存失败");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path)
            .expect("读权限")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "设置文件应仅本人可读写");
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn corrupt_file_falls_back_with_warning() {
    let path = temp_path("corrupt.json");
    std::fs::write(&path, b"{ not json").expect("写入测试文件失败");
    let (settings, warning) = load(&path);
    assert_eq!(settings, Settings::default());
    assert!(warning.unwrap_or_default().contains("格式有误"));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn partial_and_unknown_fields_are_tolerated() {
    let path = temp_path("partial.json");
    std::fs::write(
        &path,
        br#"{"preferredQuality":"original","futureField":42,"windowWidth":-5}"#,
    )
    .expect("写入测试文件失败");
    let (settings, warning) = load(&path);
    assert!(warning.is_none());
    assert_eq!(settings.preferred_quality, Quality::Original);
    assert_eq!(
        settings.player_backend,
        PlayerBackend::Embedded,
        "旧设置没有播放器预设字段时默认内嵌"
    );
    assert!(settings.notify_enabled, "缺失字段取默认");
    assert!(settings.auto_subtitles, "旧设置没有这个字段时默认开");
    assert_eq!(
        settings.player_audio_lang,
        PreferredLanguage::Auto,
        "旧设置没有首选音轨语言时跟随系统"
    );
    assert_eq!(
        settings.player_sub_lang,
        PreferredLanguage::Auto,
        "旧设置没有首选字幕语言时跟随系统"
    );
    assert!(
        settings.player_mpv_conf,
        "旧设置没有 embed-mpv.conf 字段时默认开"
    );
    assert!(
        settings.player_input_conf,
        "旧设置没有 embed-input.conf 字段时默认开"
    );
    assert!(
        settings.player_vulkan,
        "旧设置没有渲染后端字段时默认 Vulkan"
    );
    assert_eq!(settings.player_volume, 100, "旧设置没有音量字段时满音量");
    assert!(!settings.player_muted, "旧设置没有静音字段时不静音");
    assert_eq!(settings.player_speed_percent, 100, "旧设置没有倍速时原速");
    assert_eq!(settings.player_sub_font_size, DEFAULT_SUB_FONT_SIZE);
    assert_eq!(settings.player_sub_pos, DEFAULT_SUB_POS);
    assert_eq!(settings.player_sub_color, DEFAULT_SUB_COLOR);
    assert_eq!(settings.player_sub_outline_size, DEFAULT_SUB_OUTLINE_SIZE);
    assert_eq!(settings.player_sub_outline_color, DEFAULT_SUB_OUTLINE_COLOR);
    assert_eq!(
        settings.content_view,
        ContentViewMode::Grid,
        "旧设置没有内容视图字段时默认网格"
    );
    assert_eq!(settings.window_width, 0, "负尺寸收回为 0");
    assert_eq!(settings.download_parallel, 3, "旧设置没有并发字段时取默认");
    assert_eq!(
        settings.download_connections, 4,
        "旧设置没有分片字段时取默认"
    );
    assert_eq!(settings.download_rate_mib, 0, "旧设置默认不限速");
    assert_eq!(
        settings.upload_parallel, 2,
        "旧设置没有上传并发字段时取默认"
    );
    assert_eq!(settings.upload_rate_mib, 0, "旧设置上传默认不限速");
    assert!(settings.auto_resume_downloads, "自动续跑默认开启");
    assert!(!settings.auto_resume_uploads, "上传自动续传默认关闭");
    assert!(
        !settings.local_index_enabled,
        "本地目录索引默认关闭（旧设置没有这个字段时同样关闭）"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn legacy_custom_player_becomes_custom_backend() {
    let path = temp_path("legacy-player.json");
    std::fs::write(
        &path,
        br#"{"version":1,"playerCommand":"vlc","playerArgs":"--fullscreen"}"#,
    )
    .expect("写入旧设置失败");
    let (settings, warning) = load(&path);
    assert!(warning.is_none());
    assert_eq!(settings.player_backend, PlayerBackend::Custom);
    assert_eq!(settings.version, SETTINGS_VERSION);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn legacy_custom_args_without_a_command_stay_custom() {
    let path = temp_path("legacy-player-args.json");
    std::fs::write(&path, br#"{"version":1,"playerArgs":"--fullscreen"}"#).expect("写入旧设置失败");
    let (settings, warning) = load(&path);
    assert!(warning.is_none());
    assert_eq!(settings.player_backend, PlayerBackend::Custom);
    assert_eq!(settings.player_args, "--fullscreen");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn legacy_file_view_setting_migrates_to_content_view() {
    let path = temp_path("legacy-file-view.json");
    std::fs::write(&path, br#"{"fileView":"grid"}"#).expect("写入旧设置失败");
    let (settings, warning) = load(&path);
    assert!(warning.is_none());
    assert_eq!(
        settings.content_view,
        ContentViewMode::Grid,
        "旧 fileView 应继续生效"
    );

    save(&path, &settings).expect("保存迁移后的设置失败");
    let body = std::fs::read_to_string(&path).expect("读取迁移后的设置失败");
    assert!(body.contains("\"contentView\""));
    assert!(!body.contains("\"fileView\""));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn throughput_limits_are_clamped_when_loaded() {
    let normalized = Settings {
        download_parallel: 99,
        download_connections: 0,
        download_rate_mib: u64::MAX,
        upload_parallel: 99,
        upload_rate_mib: u64::MAX,
        player_volume: 300,
        player_speed_percent: 999,
        player_sub_font_size: 1,
        player_sub_pos: 200,
        player_sub_outline_size: 99,
        player_sub_color: "white".into(),
        player_sub_outline_color: "#12".into(),
        ..Settings::default()
    }
    .normalized();
    assert_eq!(normalized.player_volume, 100, "手改音量越界时收回 100");
    assert_eq!(normalized.player_speed_percent, PLAYER_SPEED_MAX);
    assert_eq!(normalized.player_sub_font_size, SUB_FONT_SIZE_MIN);
    assert_eq!(normalized.player_sub_pos, SUB_POS_MAX);
    assert_eq!(normalized.player_sub_outline_size, SUB_OUTLINE_SIZE_MAX);
    assert_eq!(normalized.player_sub_color, DEFAULT_SUB_COLOR);
    assert_eq!(
        normalized.player_sub_outline_color,
        DEFAULT_SUB_OUTLINE_COLOR
    );
    assert_eq!(
        Settings {
            player_volume: -5,
            ..Settings::default()
        }
        .normalized()
        .player_volume,
        0
    );
    assert_eq!(
        normalized.download_parallel,
        crate::session::limits::MAX_PARALLEL
    );
    assert_eq!(normalized.download_connections, 1);
    assert_eq!(
        normalized.download_rate_mib,
        crate::session::limits::MAX_RATE_MIB
    );
    assert_eq!(
        normalized.upload_parallel,
        crate::session::limits::MAX_UPLOAD_PARALLEL
    );
    assert_eq!(
        normalized.upload_rate_mib,
        crate::session::limits::MAX_UPLOAD_RATE_MIB
    );
}

#[test]
fn download_dir_defaults_and_expands_home() {
    // 空串 → 默认目录（~/Downloads），末尾就是 Downloads
    let settings = Settings::default();
    assert!(settings.download_dir_path().ends_with("Downloads"));
    // ~/xxx 展开
    let expanded = Settings {
        download_dir: "~/Videos/kichi".into(),
        ..Settings::default()
    };
    let path = expanded.download_dir_path();
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            assert_eq!(path, PathBuf::from(home).join("Videos/kichi"));
        }
    }
    // 绝对路径原样用；前后空白不算数
    let absolute = Settings {
        download_dir: "  /tmp/kichi-dl  ".into(),
        ..Settings::default()
    };
    assert_eq!(absolute.download_dir_path(), PathBuf::from("/tmp/kichi-dl"));
}
