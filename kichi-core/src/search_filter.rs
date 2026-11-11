//! 搜索筛选：类型 / 修改时间 / 大小。
//!
//! 这里只有纯函数，不碰网络、不碰 Qt：
//! - `kind` / `modified_at` / `size` 的条件同时给出 SQL 下推所需的值
//!   （[`SearchFilter::pushdown_kind`] 等），由索引查询拼进 `WHERE`；
//! - 扩展名分类（视频 / 音频 / 图片 / 文档 / 压缩包 / 其他）在
//!   [`classify_name`] 里做，SQL 只负责 `kind = 0`，分类结果由
//!   [`SearchFilter::matches`] 再判一次；
//! - 云盘扫描与「只筛当前目录」的兜底同样直接调用 [`SearchFilter::matches`]。
//!
//! 语义约定：
//! - 每组可以多选，同组内取**并集**；空列表表示该组不限制；
//! - 修改时间缺失的条目，在启用时间筛选时**不命中**（无法判断是否在窗口内）；
//!   多选时间窗口的并集等价于其中最长的一档；
//! - 文件夹没有大小，启用大小筛选时**一律不命中**；
//! - 大小区间首尾相接、不重叠：`[0,10MiB)` / `[10MiB,100MiB)` /
//!   `[100MiB,1GiB)` / `[1GiB,∞)`，多选时 SQL 里用 `OR` 拼起来。

use crate::drive::{AUDIO_EXTENSIONS, IMAGE_EXTENSIONS, VIDEO_EXTENSIONS};
use crate::types::{EntryKind, FileEntry};

/// 一天的秒数（时间筛选按自然秒数窗口计算，不按日历日）。
pub const DAY_SECS: i64 = 86_400;
pub const MIB: u64 = 1024 * 1024;
pub const GIB: u64 = 1024 * MIB;

/// 文档类扩展名（纯文本、办公文档、电子书）。
pub const DOCUMENT_EXTENSIONS: &[&str] = &[
    "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt", "md", "markdown", "rtf", "odt",
    "ods", "odp", "csv", "epub",
];

/// 压缩包 / 镜像扩展名。
pub const ARCHIVE_EXTENSIONS: &[&str] = &[
    "zip", "rar", "7z", "tar", "gz", "tgz", "bz2", "tbz", "tbz2", "xz", "txz", "zst", "lz4", "lz",
    "iso", "cab",
];

/// 「类型」筛选。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TypeFilter {
    /// 全部（文件夹 + 文件）
    #[default]
    All,
    /// 只看文件夹
    Folder,
    /// 全部文件
    File,
    Video,
    Audio,
    Image,
    Document,
    Archive,
    /// 不属于以上任何一类的文件
    Other,
}

impl TypeFilter {
    /// QML / 状态持久化用的线名；认不出的值回退「全部」。
    pub fn parse(value: &str) -> Self {
        match value.trim() {
            "folder" => Self::Folder,
            "file" => Self::File,
            "video" => Self::Video,
            "audio" => Self::Audio,
            "image" => Self::Image,
            "document" => Self::Document,
            "archive" => Self::Archive,
            "other" => Self::Other,
            _ => Self::All,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Folder => "folder",
            Self::File => "file",
            Self::Video => "video",
            Self::Audio => "audio",
            Self::Image => "image",
            Self::Document => "document",
            Self::Archive => "archive",
            Self::Other => "other",
        }
    }

    /// 状态行摘要里的中文名。
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "全部",
            Self::Folder => "文件夹",
            Self::File => "文件",
            Self::Video => "视频",
            Self::Audio => "音频",
            Self::Image => "图片",
            Self::Document => "文档",
            Self::Archive => "压缩包",
            Self::Other => "其他",
        }
    }

    pub fn is_active(self) -> bool {
        self != Self::All
    }

    fn allows(self, entry: &FileEntry) -> bool {
        match self {
            Self::All => true,
            Self::Folder => entry.kind == EntryKind::Folder,
            Self::File => entry.kind == EntryKind::File,
            Self::Video => is_file_category(entry, FileCategory::Video),
            Self::Audio => is_file_category(entry, FileCategory::Audio),
            Self::Image => is_file_category(entry, FileCategory::Image),
            Self::Document => is_file_category(entry, FileCategory::Document),
            Self::Archive => is_file_category(entry, FileCategory::Archive),
            Self::Other => is_file_category(entry, FileCategory::Other),
        }
    }
}

/// 「修改时间」筛选。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ModifiedFilter {
    #[default]
    Any,
    Last7Days,
    Last30Days,
    LastYear,
}

impl ModifiedFilter {
    pub fn parse(value: &str) -> Self {
        match value.trim() {
            "7d" => Self::Last7Days,
            "30d" => Self::Last30Days,
            "1y" => Self::LastYear,
            _ => Self::Any,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Any => "any",
            Self::Last7Days => "7d",
            Self::Last30Days => "30d",
            Self::LastYear => "1y",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Any => "不限",
            Self::Last7Days => "最近 7 天",
            Self::Last30Days => "最近 30 天",
            Self::LastYear => "最近 1 年",
        }
    }

    pub fn is_active(self) -> bool {
        self != Self::Any
    }

    /// 窗口长度（秒）；`Any` 为 `None`。
    pub fn max_age_secs(self) -> Option<i64> {
        match self {
            Self::Any => None,
            Self::Last7Days => Some(7 * DAY_SECS),
            Self::Last30Days => Some(30 * DAY_SECS),
            Self::LastYear => Some(365 * DAY_SECS),
        }
    }
}

/// 「大小」筛选。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SizeFilter {
    #[default]
    Any,
    /// < 10 MiB
    Lt10MiB,
    /// [10 MiB, 100 MiB)
    MiB10To100,
    /// [100 MiB, 1 GiB)
    MiB100ToGiB,
    /// ≥ 1 GiB
    GiBPlus,
}

impl SizeFilter {
    pub fn parse(value: &str) -> Self {
        match value.trim() {
            "lt10m" => Self::Lt10MiB,
            "10m-100m" => Self::MiB10To100,
            "100m-1g" => Self::MiB100ToGiB,
            "gt1g" => Self::GiBPlus,
            _ => Self::Any,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Any => "any",
            Self::Lt10MiB => "lt10m",
            Self::MiB10To100 => "10m-100m",
            Self::MiB100ToGiB => "100m-1g",
            Self::GiBPlus => "gt1g",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Any => "不限",
            Self::Lt10MiB => "<10 MB",
            Self::MiB10To100 => "10–100 MB",
            Self::MiB100ToGiB => "100 MB–1 GB",
            Self::GiBPlus => ">1 GB",
        }
    }

    pub fn is_active(self) -> bool {
        self != Self::Any
    }

    /// `[下界, 上界)`；`Any` 为 `None`。
    pub fn bounds(self) -> Option<(Option<u64>, Option<u64>)> {
        match self {
            Self::Any => None,
            Self::Lt10MiB => Some((None, Some(10 * MIB))),
            Self::MiB10To100 => Some((Some(10 * MIB), Some(100 * MIB))),
            Self::MiB100ToGiB => Some((Some(100 * MIB), Some(GIB))),
            Self::GiBPlus => Some((Some(GIB), None)),
        }
    }

    fn matches(self, size: u64) -> bool {
        let Some((low, high)) = self.bounds() else {
            return true;
        };
        low.is_none_or(|low| size >= low) && high.is_none_or(|high| size < high)
    }
}

/// 一组筛选条件；默认值表示「不筛」。
///
/// 每组都可以多选，同组内取**并集**：勾了「视频」和「音频」就两类都显示；
/// 勾了「文件夹」和「视频」就是文件夹 + 视频。空列表 = 该组不限制。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SearchFilter {
    /// 选中的类别；空 = 全部
    pub types: Vec<TypeFilter>,
    /// 选中的修改时间窗口；空 = 不限。并集等价于其中最长的那一档。
    pub modified: Vec<ModifiedFilter>,
    /// 选中的大小区间；空 = 不限
    pub sizes: Vec<SizeFilter>,
}

impl SearchFilter {
    /// 从界面线名（`video` / `7d` / `gt1g`…）构造；认不出的值忽略。
    ///
    /// `all` / `any` 是「不限制」的界面写法，内部用空列表表示，不存进条件里。
    pub fn from_wire(types: &[String], modified: &[String], sizes: &[String]) -> Self {
        let mut filter = Self::default();
        for value in types {
            let kind = TypeFilter::parse(value);
            if kind.is_active() && !filter.types.contains(&kind) {
                filter.types.push(kind);
            }
        }
        for value in modified {
            let window = ModifiedFilter::parse(value);
            if window.is_active() && !filter.modified.contains(&window) {
                filter.modified.push(window);
            }
        }
        for value in sizes {
            let range = SizeFilter::parse(value);
            if range.is_active() && !filter.sizes.contains(&range) {
                filter.sizes.push(range);
            }
        }
        filter
    }

    pub fn is_active(&self) -> bool {
        self.active_count() > 0
    }

    /// 激活的条件组数（0–3），界面上的「筛选 N」用。
    pub fn active_count(&self) -> usize {
        [
            !self.types.is_empty(),
            !self.modified.is_empty(),
            !self.sizes.is_empty(),
        ]
        .into_iter()
        .filter(|active| *active)
        .count()
    }

    /// 状态行摘要，例如「视频、音频 · 最近 7 天 · 10–100 MB」；没有筛选时为空串。
    ///
    /// 同一组多选时用顿号连起来，组与组之间用「 · 」。
    pub fn summary(&self) -> String {
        let mut parts = Vec::with_capacity(3);
        if !self.types.is_empty() {
            parts.push(
                self.types
                    .iter()
                    .map(|kind| kind.label())
                    .collect::<Vec<_>>()
                    .join("、"),
            );
        }
        if !self.modified.is_empty() {
            parts.push(
                self.modified
                    .iter()
                    .map(|window| window.label())
                    .collect::<Vec<_>>()
                    .join("、"),
            );
        }
        if !self.sizes.is_empty() {
            parts.push(
                self.sizes
                    .iter()
                    .map(|range| range.label())
                    .collect::<Vec<_>>()
                    .join("、"),
            );
        }
        parts.join(" · ")
    }

    /// 一条条目是否通过全部条件。`now` 是当前 unix 秒。
    ///
    /// 组内并集：命中任一选中项即可；组为空表示不限制。
    pub fn matches(&self, entry: &FileEntry, now: i64) -> bool {
        if !self.types.is_empty() && !self.types.iter().any(|kind| kind.allows(entry)) {
            return false;
        }
        if let Some(age) = self.max_age_secs() {
            let Some(modified) = entry.modified_at.map(|time| time.timestamp()) else {
                return false;
            };
            if modified < now.saturating_sub(age) {
                return false;
            }
        }
        if !self.sizes.is_empty() {
            // 文件夹没有大小，按大小筛时一律不展示。
            if entry.kind == EntryKind::Folder {
                return false;
            }
            if !self.sizes.iter().any(|range| range.matches(entry.size)) {
                return false;
            }
        }
        true
    }

    /// 选中窗口里最长的一个（并集等价于它）；没有选时间时 `None`。
    pub fn max_age_secs(&self) -> Option<i64> {
        self.modified
            .iter()
            .filter_map(|window| window.max_age_secs())
            .max()
    }

    // —— SQL 下推：索引查询直接拼进 WHERE，减少读出来的行数 ——

    /// `kind` 约束（1 文件夹 / 0 文件）；`None` 表示两种都可能出现。
    ///
    /// 启用大小筛选时强制 `kind = 0`：文件夹没有大小，本来就不会命中。
    /// 类型里同时勾了文件夹和文件类（并集包含两种）时也不下推。
    pub fn pushdown_kind(&self) -> Option<i64> {
        if !self.sizes.is_empty() {
            return Some(0);
        }
        if self.types.is_empty() {
            return None;
        }
        if self.types.contains(&TypeFilter::Folder) {
            return (self.types.len() == 1).then_some(1);
        }
        Some(0)
    }

    /// `modified_at >= ?` 的下界（unix 秒）；`None` 表示不限。
    pub fn pushdown_modified_after(&self, now: i64) -> Option<i64> {
        self.max_age_secs().map(|age| now.saturating_sub(age))
    }

    /// 大小区间的并集（每项 `[low, high)`）；`None` 表示不限。
    ///
    /// 各组区间本身首尾相接、不重叠，调用方用 `OR` 把它们拼进 SQL。
    pub fn pushdown_size_ranges(&self) -> Option<Vec<(Option<u64>, Option<u64>)>> {
        if self.sizes.is_empty() {
            return None;
        }
        Some(
            self.sizes
                .iter()
                .filter_map(|range| range.bounds())
                .collect(),
        )
    }
}

/// 文件扩展名分类（只对文件有意义）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileCategory {
    Video,
    Audio,
    Image,
    Document,
    Archive,
    Other,
}

/// 取小写扩展名（不含点）；没有扩展名时返回 `None`。
pub fn file_extension(name: &str) -> Option<String> {
    let (_, extension) = name.rsplit_once('.')?;
    let extension = extension.trim().to_ascii_lowercase();
    (!extension.is_empty()).then_some(extension)
}

/// 按扩展名把文件名归入筛选用的类别。
///
/// 视频 / 音频 / 图片复用 [`crate::drive`] 里与播放、缩略图一致的扩展名表，
/// 避免「能播的视频却被筛选归为其他」这类分叉。
pub fn classify_name(name: &str) -> FileCategory {
    let Some(extension) = file_extension(name) else {
        return FileCategory::Other;
    };
    let extension = extension.as_str();
    if VIDEO_EXTENSIONS.contains(&extension) {
        FileCategory::Video
    } else if AUDIO_EXTENSIONS.contains(&extension) {
        FileCategory::Audio
    } else if IMAGE_EXTENSIONS.contains(&extension) {
        FileCategory::Image
    } else if DOCUMENT_EXTENSIONS.contains(&extension) {
        FileCategory::Document
    } else if ARCHIVE_EXTENSIONS.contains(&extension) {
        FileCategory::Archive
    } else {
        FileCategory::Other
    }
}

fn is_file_category(entry: &FileEntry, category: FileCategory) -> bool {
    entry.kind == EntryKind::File && classify_name(&entry.name) == category
}

#[cfg(test)]
#[path = "search_filter_tests.rs"]
mod tests;
