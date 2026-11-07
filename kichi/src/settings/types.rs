//! 设置的枚举值、语言标记与各自的宽松解析。

use kichi_core::subtitles::{CHINESE_ASCII, CHINESE_TEXT};
use kichi_core::types::Tier;
use serde::{Deserialize, Serialize};

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

pub(super) fn tags_for_locale(raw: &str) -> Option<LanguageTags> {
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

pub(super) fn is_language_code(code: &str) -> bool {
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
