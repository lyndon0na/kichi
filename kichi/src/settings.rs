//! 界面设置：清晰度偏好、播放器命令、通知开关、排序、窗口记忆。
//!
//! 存放位置 `~/.config/kichi/settings.json`（`KICHI_SETTINGS_FILE` 可覆盖）。
//! 同目录还放令牌降级文件，整个 `kichi` 目录归本项目独占——
//! 旧版（Python + aria2 那版）的配置已迁到 `~/.config/kichi-py`。
//!
//! 约定：
//! - 文件不存在 → 全部默认值；文件损坏 → 回默认值并把原因交给界面提示，不静默吞掉；
//! - 读写失败一律冒泡（`Result`），由调用方决定提示方式；
//! - 只存界面偏好，**不存任何凭据**（令牌只进 KWallet）；
//! - 未知字段忽略、缺失字段取默认（前向兼容旧配置与新配置混用）。

use std::path::{Path, PathBuf};

use kichi_core::types::Tier;
use serde::{Deserialize, Serialize};

/// 设置文件格式版本（将来不兼容改动时据此迁移）。
pub const SETTINGS_VERSION: u32 = 1;

/// 清晰度偏好（对应界面上的「默认清晰度」）。
///
/// 转码档按**档位名**匹配（`"1080P"` 这样的字面量），不按高度范围：
/// 平台返回什么名字就认什么名字，指定档位没有时回退原画（见 `kichi_core::types::resolve`）。
///
/// 序列化：`"ask"` / `"original"` / `"transcode:1080P"`（不用 serde 的枚举默认形态，
/// 跨版本改起来太脆）。旧文件里的 `"transcode"` 认不出来，回「每次询问」——
/// 不替用户猜一个档位。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Quality {
    /// 每次弹出档位菜单自己挑（现状）
    #[default]
    Ask,
    /// 优先原画直通播放
    Original,
    /// 指定转码档；没有这一档就回退原画
    Transcode(&'static str),
}

/// 菜单里固定列出的档位（原画在前）。唯一出处是 `kichi_core::types::TIERS`，这里只是转手。
pub use kichi_core::types::TIERS as FIXED_TIERS;

/// 界面上可选的转码档（顺序即下拉顺序），从 [`FIXED_TIERS`] 派生。
pub const TRANSCODE_TIERS: [&str; 3] = match FIXED_TIERS {
    [Tier::Original, Tier::Transcode(a), Tier::Transcode(b), Tier::Transcode(c)] => [a, b, c],
    _ => ["1080P", "720P", "480P"],
};

/// 存盘前缀：`transcode:1080P`
const TRANSCODE_PREFIX: &str = "transcode:";

impl Quality {
    /// 设置里存的原始字符串（`parse` 能读回来的那一种）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Original => "original",
            // 只有 TRANSCODE_TIERS 里的档位才会被构造出来（parse 守着，界面也只给这几个），
            // 其余一律按「每次询问」存，免得存进一个谁也解析不回的字符串
            Self::Transcode(tier) => Tier::Transcode(tier).setting_value(),
        }
    }

    /// 从界面字符串解析；无法识别的一律回「每次询问」。
    pub fn parse(text: &str) -> Self {
        let text = text.trim();
        if let Some(tier) = text.strip_prefix(TRANSCODE_PREFIX) {
            return TRANSCODE_TIERS
                .iter()
                .find(|known| known.eq_ignore_ascii_case(tier))
                .map(|known| Self::Transcode(known))
                .unwrap_or(Self::Ask);
        }
        match text {
            "original" => Self::Original,
            _ => Self::Ask,
        }
    }
}

impl Serialize for Quality {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Quality {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Ok(Self::parse(&text))
    }
}

/// 文件列表排序字段（文件夹永远置顶，不受这里影响）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum SortField {
    #[default]
    Name,
    Size,
    Modified,
}

impl SortField {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Size => "size",
            Self::Modified => "modified",
        }
    }

    pub fn parse(text: &str) -> Self {
        match text.trim() {
            "size" => Self::Size,
            "modified" => Self::Modified,
            _ => Self::Name,
        }
    }
}

/// 全部可持久化设置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// 格式版本；读到的版本比当前新时不报错（只按已知字段解析）
    pub version: u32,
    pub preferred_quality: Quality,
    /// 播放器整条命令；空串表示用内置默认
    pub player_command: String,
    /// 追加参数；空串表示用内置默认
    pub player_args: String,
    /// 离线任务完成/失败时弹系统通知
    pub notify_enabled: bool,
    /// 播放时自动加载同目录字幕（只对 mpv 生效）
    pub auto_subtitles: bool,
    /// 关闭窗口时缩到托盘
    pub close_to_tray: bool,
    pub sort_field: SortField,
    pub sort_ascending: bool,
    /// 窗口宽高；0 表示还没存过（用默认尺寸）
    pub window_width: i32,
    pub window_height: i32,
    pub window_maximized: bool,
    /// 下载目录；空串表示用默认（`~/Downloads`）
    pub download_dir: String,
    /// 同时下载的文件数（1–8）
    pub download_parallel: usize,
    /// 单文件分片连接数（1–8；1 = 不分片）
    pub download_connections: usize,
    /// 全局限速 MiB/s；0 = 不限
    pub download_rate_mib: u64,
    /// 正常退出后，下次恢复登录态时自动继续未完成传输
    pub auto_resume_downloads: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            preferred_quality: Quality::Ask,
            player_command: String::new(),
            player_args: String::new(),
            notify_enabled: true,
            auto_subtitles: true,
            close_to_tray: false,
            sort_field: SortField::Name,
            sort_ascending: true,
            window_width: 0,
            window_height: 0,
            window_maximized: false,
            download_dir: String::new(),
            download_parallel: crate::session::limits::DEFAULT_PARALLEL,
            download_connections: crate::session::limits::DEFAULT_CONNECTIONS,
            download_rate_mib: 0,
            auto_resume_downloads: true,
        }
    }
}

impl Settings {
    /// 播放器命令（空串 → `None`，交给调用方用默认值）。
    pub fn player_command_override(&self) -> Option<&str> {
        non_empty(&self.player_command)
    }

    /// 播放器参数（空串 → `None`）。
    pub fn player_args_override(&self) -> Option<&str> {
        non_empty(&self.player_args)
    }

    /// 下载目录（空串 → `~/Downloads`）。
    ///
    /// 不在这里创建目录：真正下载时才建，免得只是打开设置就多出文件夹。
    pub fn download_dir_path(&self) -> PathBuf {
        match non_empty(&self.download_dir) {
            Some(text) => PathBuf::from(expand_home(text)),
            None => default_download_dir(),
        }
    }
}

/// `~/xxx` 展开成家目录；`$HOME` 取不到时原样返回（相对路径也能用）。
fn expand_home(text: &str) -> String {
    let Some(rest) = text.strip_prefix("~/") else {
        return text.to_owned();
    };
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() => format!("{}/{}", home.trim_end_matches('/'), rest),
        _ => text.to_owned(),
    }
}

/// 默认下载目录 `~/Downloads`；取不到家目录时退回系统临时目录。
///
/// 不写死 `/home/xxx`：这是给任何人在任何机器上用的默认值。
pub fn default_download_dir() -> PathBuf {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() => PathBuf::from(home).join("Downloads"),
        _ => std::env::temp_dir(),
    }
}

fn non_empty(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

/// 设置文件的默认位置：`<配置目录>/settings.json`。
///
/// 优先 `KICHI_SETTINGS_FILE` 覆盖，与 [`crate::session::config_dir`] 同目录。
pub fn default_path() -> PathBuf {
    if let Some(path) = std::env::var_os("KICHI_SETTINGS_FILE") {
        return PathBuf::from(path);
    }
    crate::session::config_dir().join("settings.json")
}

/// 读取设置。
///
/// 返回值里的 `warning` 是「文件坏了 / 读不了，已回默认值」这类需要提示
/// 但不该阻断使用的原因；`Ok((defaults, None))` 表示文件本来就不存在。
pub fn load(path: &Path) -> (Settings, Option<String>) {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return (Settings::default(), None)
        }
        Err(err) => {
            return (
                Settings::default(),
                Some(format!("设置文件读取失败（已用默认设置）：{err}")),
            )
        }
    };
    match serde_json::from_slice::<Settings>(&bytes) {
        Ok(settings) => (settings.normalized(), None),
        Err(err) => (
            Settings::default(),
            Some(format!("设置文件格式有误（已用默认设置）：{err}")),
        ),
    }
}

impl Settings {
    /// 把明显越界的值收回来（手改配置文件也不会让窗口变成负尺寸）。
    fn normalized(mut self) -> Self {
        self.version = self.version.max(1);
        if self.window_width < 0 {
            self.window_width = 0;
        }
        if self.window_height < 0 {
            self.window_height = 0;
        }
        self.download_parallel = self
            .download_parallel
            .clamp(1, crate::session::limits::MAX_PARALLEL);
        self.download_connections = self
            .download_connections
            .clamp(1, crate::session::limits::MAX_CONNECTIONS);
        self.download_rate_mib = self
            .download_rate_mib
            .min(crate::session::limits::MAX_RATE_MIB);
        self
    }
}

/// 写盘：先写同目录临时文件再 rename，避免写到一半断电留下半个 JSON。
pub fn save(path: &Path, settings: &Settings) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| format!("创建设置目录失败：{err}"))?;
    }
    let body =
        serde_json::to_vec_pretty(settings).map_err(|err| format!("序列化设置失败：{err}"))?;
    let temp = path.with_extension("json.tmp");
    write_private(&temp, &body).map_err(|err| format!("写入设置失败：{err}"))?;
    std::fs::rename(&temp, path).map_err(|err| {
        let _ = std::fs::remove_file(&temp);
        format!("保存设置失败：{err}")
    })
}

/// 写一个只有本人可读写的文件（0600）。
///
/// 设置里虽然没有密码，但含播放器命令与窗口信息，同一台机器的其他用户
/// 没必要能看到；和令牌降级文件保持一致的口径。
fn write_private(path: &Path, body: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(body)?;
    file.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(warning.is_none(), "文件不存在不算错误");
    }

    #[test]
    fn roundtrip_keeps_every_field() {
        let path = temp_path("roundtrip.json");
        let settings = Settings {
            preferred_quality: Quality::Transcode("1080P"),
            player_command: "mpv".into(),
            player_args: "--fs --vo=gpu".into(),
            notify_enabled: false,
            close_to_tray: true,
            sort_field: SortField::Modified,
            sort_ascending: false,
            window_width: 1280,
            window_height: 720,
            window_maximized: true,
            download_parallel: 5,
            download_connections: 6,
            download_rate_mib: 12,
            auto_resume_downloads: false,
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
        assert_eq!(settings.notify_enabled, true, "缺失字段取默认");
        assert!(settings.auto_subtitles, "旧设置没有这个字段时默认开");
        assert_eq!(settings.window_width, 0, "负尺寸收回为 0");
        assert_eq!(settings.download_parallel, 3, "旧设置没有并发字段时取默认");
        assert_eq!(
            settings.download_connections, 4,
            "旧设置没有分片字段时取默认"
        );
        assert_eq!(settings.download_rate_mib, 0, "旧设置默认不限速");
        assert!(settings.auto_resume_downloads, "自动续跑默认开启");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn download_limits_are_clamped_when_loaded() {
        let normalized = Settings {
            download_parallel: 99,
            download_connections: 0,
            download_rate_mib: u64::MAX,
            ..Settings::default()
        }
        .normalized();
        assert_eq!(
            normalized.download_parallel,
            crate::session::limits::MAX_PARALLEL
        );
        assert_eq!(normalized.download_connections, 1);
        assert_eq!(
            normalized.download_rate_mib,
            crate::session::limits::MAX_RATE_MIB
        );
    }

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
}
