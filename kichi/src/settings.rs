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

use serde::{Deserialize, Serialize};

/// 设置文件格式版本（将来不兼容改动时据此迁移）。
pub const SETTINGS_VERSION: u32 = 1;

/// 清晰度偏好（对应界面上的「默认清晰度」）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Quality {
    /// 每次弹出档位菜单自己挑（现状）
    #[default]
    Ask,
    /// 优先原画直通播放
    Original,
    /// 优先转码档（最高清晰度那档）直通播放
    Transcode,
}

impl Quality {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Original => "original",
            Self::Transcode => "transcode",
        }
    }

    /// 从界面字符串解析；无法识别的一律回「每次询问」。
    pub fn parse(text: &str) -> Self {
        match text.trim() {
            "original" => Self::Original,
            "transcode" => Self::Transcode,
            _ => Self::Ask,
        }
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
    /// 关闭窗口时缩到托盘
    pub close_to_tray: bool,
    pub sort_field: SortField,
    pub sort_ascending: bool,
    /// 窗口宽高；0 表示还没存过（用默认尺寸）
    pub window_width: i32,
    pub window_height: i32,
    pub window_maximized: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            preferred_quality: Quality::Ask,
            player_command: String::new(),
            player_args: String::new(),
            notify_enabled: true,
            close_to_tray: false,
            sort_field: SortField::Name,
            sort_ascending: true,
            window_width: 0,
            window_height: 0,
            window_maximized: false,
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
            preferred_quality: Quality::Transcode,
            player_command: "mpv".into(),
            player_args: "--fs --vo=gpu".into(),
            notify_enabled: false,
            close_to_tray: true,
            sort_field: SortField::Modified,
            sort_ascending: false,
            window_width: 1280,
            window_height: 720,
            window_maximized: true,
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
        assert_eq!(settings.window_width, 0, "负尺寸收回为 0");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn enum_parsing_is_lenient() {
        assert_eq!(Quality::parse("original"), Quality::Original);
        assert_eq!(Quality::parse("transcode "), Quality::Transcode);
        assert_eq!(Quality::parse("nonsense"), Quality::Ask);
        assert_eq!(SortField::parse("size"), SortField::Size);
        assert_eq!(SortField::parse(""), SortField::Name);
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
}
