//! 界面设置：清晰度偏好、播放器命令、通知开关、文件视图、排序、窗口记忆。
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
//!
//! 按职责拆分：本文件是 `Settings` 本体、默认值、迁移与读写；枚举 / 语言标记
//! 在 [`types`]，测试按「读写迁移」和「枚举解析」分开。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::session::private_fs;

#[cfg(test)]
mod tests;
mod types;

pub use types::{
    ContentViewMode, PlayerBackend, PreferredLanguage, Quality, SortField, FIXED_TIERS,
};

pub const SETTINGS_VERSION: u32 = 3;

/// 内嵌播放器倍速范围（百分比；100 = 原速）。
pub const PLAYER_SPEED_MIN: u16 = 50;
pub const PLAYER_SPEED_MAX: u16 = 300;
/// 字幕字号范围（mpv 的 `sub-font-size`）。
pub const SUB_FONT_SIZE_MIN: u16 = 16;
pub const SUB_FONT_SIZE_MAX: u16 = 100;
/// mpv 的 `sub-pos`：0 = 顶部，100 = 底部，上限 150。
pub const SUB_POS_MAX: u16 = 150;
/// 描边粗细上限（mpv 的 `sub-outline-size`）。
pub const SUB_OUTLINE_SIZE_MAX: u16 = 8;

/// 与 mpv 的 `sub-font-size` 默认值一致：迁移到内嵌播放时字幕大小不变。
pub const DEFAULT_SUB_FONT_SIZE: u16 = 55;
pub const DEFAULT_SUB_POS: u16 = 100;
pub const DEFAULT_SUB_OUTLINE_SIZE: u16 = 2;
pub const DEFAULT_SUB_COLOR: &str = "#FFFFFFFF";
pub const DEFAULT_SUB_OUTLINE_COLOR: &str = "#FF000000";

/// 全部可持久化设置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// 格式版本；读到的版本比当前新时不报错（只按已知字段解析）
    pub version: u32,
    pub preferred_quality: Quality,
    /// 播放器预设：内嵌 / mpv / VLC / 自定义命令
    pub player_backend: PlayerBackend,
    /// 播放器整条命令；空串表示用内置默认
    pub player_command: String,
    /// 追加参数；空串表示用内置默认
    pub player_args: String,
    /// 离线任务完成/失败时弹系统通知
    pub notify_enabled: bool,
    /// 播放时自动加载同目录字幕（只对 mpv 生效）
    pub auto_subtitles: bool,
    /// 播完自动播放同文件夹下一集（默认开启）
    pub auto_play_next: bool,
    /// 内嵌播放器首选音轨语言（`auto` / `zh` / `yue` / `en` / `ja` / `ko`）
    pub player_audio_lang: PreferredLanguage,
    /// 内嵌播放器首选字幕语言
    pub player_sub_lang: PreferredLanguage,
    /// 内嵌播放器读取自己的 `embed-mpv.conf`（文件不存在时无效果）
    pub player_mpv_conf: bool,
    /// 内嵌播放器加载自己的 `embed-input.conf`（文件不存在时无效果）
    pub player_input_conf: bool,
    /// 内嵌播放器使用 Vulkan 渲染（关闭 = OpenGL）
    pub player_vulkan: bool,
    /// 内嵌播放器音量（0–100，跨视频 / 跨启动记住）
    pub player_volume: i32,
    /// 内嵌播放器静音状态
    pub player_muted: bool,
    /// 内嵌播放器倍速（百分比，50–300；跨视频 / 跨启动记住）
    pub player_speed_percent: u16,
    /// 字幕字号（mpv `sub-font-size`）
    pub player_sub_font_size: u16,
    /// 字幕垂直位置（mpv `sub-pos`：0 顶部 / 100 底部）
    pub player_sub_pos: u16,
    /// 字幕文字颜色（`#AARRGGBB`）
    pub player_sub_color: String,
    /// 字幕描边粗细（mpv `sub-outline-size`）
    pub player_sub_outline_size: u16,
    /// 字幕描边 / 边框颜色（`#AARRGGBB`）
    pub player_sub_outline_color: String,
    /// 关闭窗口时缩到托盘
    pub close_to_tray: bool,
    /// 文件类内容页共用显示方式：列表 / 网格
    #[serde(alias = "fileView")]
    pub content_view: ContentViewMode,
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
    /// 同时上传的文件数（1–8）
    pub upload_parallel: usize,
    /// 上传全局限速 MiB/s；0 = 不限
    pub upload_rate_mib: u64,
    /// 正常退出后，下次恢复登录态时自动继续未完成的下载
    pub auto_resume_downloads: bool,
    /// 正常退出后，下次恢复登录态时自动继续未完成的上传（默认关闭）
    pub auto_resume_uploads: bool,
    /// 在本机保存目录树索引与页面快照（默认关闭）
    pub local_index_enabled: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            preferred_quality: Quality::Ask,
            player_backend: PlayerBackend::default(),
            player_command: String::new(),
            player_args: String::new(),
            notify_enabled: true,
            auto_subtitles: true,
            auto_play_next: true,
            player_audio_lang: PreferredLanguage::default(),
            player_sub_lang: PreferredLanguage::default(),
            player_mpv_conf: true,
            player_input_conf: true,
            player_vulkan: true,
            player_volume: 100,
            player_muted: false,
            player_speed_percent: 100,
            player_sub_font_size: DEFAULT_SUB_FONT_SIZE,
            player_sub_pos: DEFAULT_SUB_POS,
            player_sub_color: DEFAULT_SUB_COLOR.to_owned(),
            player_sub_outline_size: DEFAULT_SUB_OUTLINE_SIZE,
            player_sub_outline_color: DEFAULT_SUB_OUTLINE_COLOR.to_owned(),
            close_to_tray: false,
            content_view: ContentViewMode::Grid,
            sort_field: SortField::Name,
            sort_ascending: true,
            window_width: 0,
            window_height: 0,
            window_maximized: false,
            download_dir: String::new(),
            download_parallel: crate::session::limits::DEFAULT_PARALLEL,
            download_connections: crate::session::limits::DEFAULT_CONNECTIONS,
            download_rate_mib: 0,
            upload_parallel: crate::session::limits::DEFAULT_UPLOAD_PARALLEL,
            upload_rate_mib: 0,
            auto_resume_downloads: true,
            auto_resume_uploads: false,
            local_index_enabled: false,
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

/// mpv 颜色只接受 `#RRGGBB` / `#AARRGGBB`。
pub fn valid_color(text: &str) -> bool {
    let bytes = text.as_bytes();
    matches!(bytes.len(), 7 | 9) && bytes[0] == b'#' && bytes[1..].iter().all(u8::is_ascii_hexdigit)
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
        // v1 只有自定义命令；新字段缺失时把旧自定义命令继续当成自定义，
        // 不能让一次升级把用户原来的播放器悄悄换回默认 mpv。
        if self.version < 2
            && (self.player_command_override().is_some() || self.player_args_override().is_some())
        {
            self.player_backend = PlayerBackend::Custom;
        }
        // v2 及更早的「mpv」是外部播放器预设；v3 起默认内嵌。没填过自定义命令的
        // 旧配置跟着升级，填过的（已在上一步变成 Custom）保持外部播放。
        if self.version < 3
            && self.player_backend == PlayerBackend::Mpv
            && self.player_command_override().is_none()
            && self.player_args_override().is_none()
        {
            self.player_backend = PlayerBackend::Embedded;
        }
        self.version = self.version.max(SETTINGS_VERSION);
        if self.window_width < 0 {
            self.window_width = 0;
        }
        if self.window_height < 0 {
            self.window_height = 0;
        }
        self.player_volume = self.player_volume.clamp(0, 100);
        self.player_speed_percent = self
            .player_speed_percent
            .clamp(PLAYER_SPEED_MIN, PLAYER_SPEED_MAX);
        self.player_sub_font_size = self
            .player_sub_font_size
            .clamp(SUB_FONT_SIZE_MIN, SUB_FONT_SIZE_MAX);
        self.player_sub_pos = self.player_sub_pos.min(SUB_POS_MAX);
        self.player_sub_outline_size = self.player_sub_outline_size.min(SUB_OUTLINE_SIZE_MAX);
        if !valid_color(&self.player_sub_color) {
            self.player_sub_color = DEFAULT_SUB_COLOR.to_owned();
        }
        if !valid_color(&self.player_sub_outline_color) {
            self.player_sub_outline_color = DEFAULT_SUB_OUTLINE_COLOR.to_owned();
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
        self.upload_parallel = self
            .upload_parallel
            .clamp(1, crate::session::limits::MAX_UPLOAD_PARALLEL);
        self.upload_rate_mib = self
            .upload_rate_mib
            .min(crate::session::limits::MAX_UPLOAD_RATE_MIB);
        self
    }

    /// 本次运行实际生效的播放器。
    ///
    /// Flatpak 沙箱里只有内嵌播放器：即使配置文件（与原生构建共用）
    /// 里还留着 mpv / VLC / 自定义，也只按内嵌执行。这里只改读取结果，
    /// **不回写** `player_backend`——否则在 Flatpak 里改一次别的设置，
    /// 就会把原生构建选好的外部播放器一起覆盖掉。
    pub fn effective_player_backend(&self) -> PlayerBackend {
        self.effective_player_backend_in(crate::platform::flatpak_sandbox())
    }

    fn effective_player_backend_in(&self, sandboxed: bool) -> PlayerBackend {
        if sandboxed {
            PlayerBackend::Embedded
        } else {
            self.player_backend
        }
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
    private_fs::write_private(&temp, &body).map_err(|err| format!("写入设置失败：{err}"))?;
    private_fs::replace_file(&temp, path).map_err(|err| format!("保存设置失败：{err}"))
}
