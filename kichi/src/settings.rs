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

use std::path::{Path, PathBuf};

use kichi_core::subtitles::{CHINESE_ASCII, CHINESE_TEXT};
use kichi_core::types::Tier;
use serde::{Deserialize, Serialize};

use crate::session::private_fs;

/// 设置文件格式版本（将来不兼容改动时据此迁移）。
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

/// 文件类内容页共用的显示方式。
///
/// 我的文件、回收站和三个资料库页共用这一项；任务 / 分享管理页仍用表格。
/// 列表是默认值；网格只改变展示，不改变数据、排序、筛选或操作。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ContentViewMode {
    #[default]
    List,
    Grid,
}

impl ContentViewMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Grid => "grid",
        }
    }

    pub fn parse(text: &str) -> Self {
        if text.trim().eq_ignore_ascii_case("grid") {
            Self::Grid
        } else {
            Self::List
        }
    }
}

impl Serialize for ContentViewMode {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ContentViewMode {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Ok(Self::parse(&text))
    }
}

/// 播放器预设。
///
/// `Custom` 继续使用 `player_command` / `player_args`；预设项忽略这两项，
/// 避免界面选了 VLC 却被旧的自定义命令悄悄接管。
/// `Embedded` 用进程内 libmpv 画在本窗口里；不可用时退回系统 mpv。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlayerBackend {
    #[default]
    Embedded,
    Mpv,
    Vlc,
    Custom,
}

impl PlayerBackend {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Embedded => "embedded",
            Self::Mpv => "mpv",
            Self::Vlc => "vlc",
            Self::Custom => "custom",
        }
    }

    pub fn parse(text: &str) -> Self {
        match text.trim().to_ascii_lowercase().as_str() {
            "embedded" => Self::Embedded,
            "vlc" => Self::Vlc,
            "custom" => Self::Custom,
            _ => Self::Mpv,
        }
    }
}

impl Serialize for PlayerBackend {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for PlayerBackend {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Ok(Self::parse(&text))
    }
}

/// 内嵌播放器的首选语言。
///
/// `Auto` 每次开播时读取 Linux 系统语言（`LC_ALL` → `LC_MESSAGES` → `LANG` →
/// `LANGUAGE`）；认不出（只有 `C` / `POSIX`）时退回中文优先。
/// 设置里只存用户选的档位，不写系统语言，换系统语言后下次开播即生效。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PreferredLanguage {
    #[default]
    Auto,
    Chinese,
    Cantonese,
    English,
    Japanese,
    Korean,
}

/// 展开后的语言标记。
///
/// `mpv` 是给 `alang` / `slang` 的 IETF 标签（逗号列表），`names` 是外挂字幕
/// 文件名匹配用的标记（可含「简体」「中字」这类非 ASCII 词）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LanguageTags {
    pub mpv: Vec<String>,
    pub names: Vec<String>,
}

impl LanguageTags {
    fn new(mpv: &[&str], names: &[&str]) -> Self {
        Self {
            mpv: mpv.iter().map(|tag| (*tag).to_owned()).collect(),
            names: names.iter().map(|tag| (*tag).to_owned()).collect(),
        }
    }

    fn generic(code: &str) -> Self {
        Self {
            mpv: vec![code.to_owned()],
            names: vec![code.to_owned()],
        }
    }

    /// mpv 的逗号列表；空列表返回 `None`（不写选项，保持 mpv 默认）。
    pub fn mpv_value(&self) -> Option<String> {
        (!self.mpv.is_empty()).then(|| self.mpv.join(","))
    }
}

impl PreferredLanguage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Chinese => "zh",
            Self::Cantonese => "yue",
            Self::English => "en",
            Self::Japanese => "ja",
            Self::Korean => "ko",
        }
    }

    pub fn parse(text: &str) -> Self {
        match text.trim().to_ascii_lowercase().as_str() {
            "zh" => Self::Chinese,
            "yue" => Self::Cantonese,
            "en" => Self::English,
            "ja" => Self::Japanese,
            "ko" => Self::Korean,
            _ => Self::Auto,
        }
    }

    /// 展开成语言标记；`Auto` 跟随系统语言，认不出时返回空。
    pub fn tags(self) -> LanguageTags {
        match self {
            Self::Auto => system_language_tags().unwrap_or_default(),
            Self::Chinese => chinese_tags(),
            Self::Cantonese => LanguageTags::new(
                &["yue", "zh-yue"],
                &["yue", "cantonese", "粤语", "粵語", "粤", "粵"],
            ),
            Self::English => {
                LanguageTags::new(&["en", "eng"], &["en", "eng", "english", "英语", "英文"])
            }
            Self::Japanese => LanguageTags::new(
                &["ja", "jpn"],
                &["ja", "jpn", "japanese", "日语", "日文", "日本語"],
            ),
            Self::Korean => LanguageTags::new(
                &["ko", "kor"],
                &[
                    "ko",
                    "kor",
                    "korean",
                    "韩语",
                    "韓語",
                    "韩文",
                    "韓文",
                    "한국어",
                ],
            ),
        }
    }
}

fn chinese_tags() -> LanguageTags {
    let mut names = CHINESE_ASCII.to_vec();
    names.extend_from_slice(CHINESE_TEXT);
    LanguageTags::new(&["zh", "chi", "zho", "cmn"], &names)
}

/// 依次读 `LC_ALL` / `LC_MESSAGES` / `LANG` / `LANGUAGE`，取第一个认得出的语言。
///
/// `LANGUAGE` 是冒号分隔的候选列表；其它变量通常只有一个值，按同样方式切开无副作用。
fn system_language_tags() -> Option<LanguageTags> {
    for name in ["LC_ALL", "LC_MESSAGES", "LANG", "LANGUAGE"] {
        let Ok(value) = std::env::var(name) else {
            continue;
        };
        for raw in value.split(':') {
            if let Some(tags) = tags_for_locale(raw) {
                return Some(tags);
            }
        }
    }
    None
}

fn tags_for_locale(raw: &str) -> Option<LanguageTags> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    // zh_CN.UTF-8 / zh-TW / sr_RS@latin 都只取语言部分。
    let base = raw
        .split(['.', '@'])
        .next()
        .unwrap_or(raw)
        .replace('-', "_");
    let code = base.split('_').next().unwrap_or("").to_ascii_lowercase();
    match code.as_str() {
        "" | "c" | "posix" => None,
        "zh" => Some(chinese_tags()),
        "yue" => Some(PreferredLanguage::Cantonese.tags()),
        "en" => Some(PreferredLanguage::English.tags()),
        "ja" => Some(PreferredLanguage::Japanese.tags()),
        "ko" => Some(PreferredLanguage::Korean.tags()),
        other if is_language_code(other) => Some(LanguageTags::generic(other)),
        _ => None,
    }
}

fn is_language_code(code: &str) -> bool {
    (2..=3).contains(&code.len()) && code.chars().all(|ch| ch.is_ascii_lowercase())
}

impl Serialize for PreferredLanguage {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for PreferredLanguage {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Ok(Self::parse(&text))
    }
}

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
    /// 内嵌播放器首选音轨语言（`auto` / `zh` / `yue` / `en` / `ja` / `ko`）
    pub player_audio_lang: PreferredLanguage,
    /// 内嵌播放器首选字幕语言
    pub player_sub_lang: PreferredLanguage,
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
            player_audio_lang: PreferredLanguage::default(),
            player_sub_lang: PreferredLanguage::default(),
            player_volume: 100,
            player_muted: false,
            player_speed_percent: 100,
            player_sub_font_size: DEFAULT_SUB_FONT_SIZE,
            player_sub_pos: DEFAULT_SUB_POS,
            player_sub_color: DEFAULT_SUB_COLOR.to_owned(),
            player_sub_outline_size: DEFAULT_SUB_OUTLINE_SIZE,
            player_sub_outline_color: DEFAULT_SUB_OUTLINE_COLOR.to_owned(),
            close_to_tray: false,
            content_view: ContentViewMode::List,
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
            player_backend: PlayerBackend::Custom,
            player_command: "mpv".into(),
            player_args: "--fs --vo=gpu".into(),
            notify_enabled: false,
            player_audio_lang: PreferredLanguage::Japanese,
            player_sub_lang: PreferredLanguage::Chinese,
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
        assert_eq!(settings.notify_enabled, true, "缺失字段取默认");
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
            ContentViewMode::List,
            "旧设置没有内容视图字段时默认列表"
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
        std::fs::write(&path, br#"{"version":1,"playerArgs":"--fullscreen"}"#)
            .expect("写入旧设置失败");
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
