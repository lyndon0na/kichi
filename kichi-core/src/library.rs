//! 资料库三页共用的领域形状。
//!
//! 这里只描述界面需要的统一条目，不包含网络请求。最近保存、播放历史和
//! 已加星标可以分别从事件或文件列表产出 [`LibraryEntry`]，界面只消费这一种形状。

use chrono::{DateTime, Utc};

use crate::{FileEntry, PlaybackProgress};

/// 侧栏资料库页面的类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryKind {
    RecentSaved,
    PlaybackHistory,
    Starred,
}

impl LibraryKind {
    /// QML 与桥接层使用的稳定键值。
    pub fn key(self) -> &'static str {
        match self {
            Self::RecentSaved => "recent-saved",
            Self::PlaybackHistory => "playback-history",
            Self::Starred => "starred",
        }
    }

    /// 只接受界面公开的三种固定值。
    pub fn from_key(value: &str) -> Option<Self> {
        match value.trim() {
            "recent-saved" => Some(Self::RecentSaved),
            "playback-history" => Some(Self::PlaybackHistory),
            "starred" => Some(Self::Starred),
            _ => None,
        }
    }
}

/// 一条资料库记录：文件本体 + 页面需要的排序时间 / 播放进度。
#[derive(Debug, Clone)]
pub struct LibraryEntry {
    pub entry: FileEntry,
    /// 官方事件 id；文件列表来源的页面为空
    pub event_id: Option<String>,
    /// 事件更新时间；没有事件时回退文件自身时间
    pub occurred_at: Option<DateTime<Utc>>,
    /// 播放历史页使用；其它页面为空
    pub progress: Option<PlaybackProgress>,
}

impl LibraryEntry {
    pub fn new(entry: FileEntry) -> Self {
        Self {
            entry,
            event_id: None,
            occurred_at: None,
            progress: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::LibraryKind;

    #[test]
    fn keys_round_trip() {
        for kind in [
            LibraryKind::RecentSaved,
            LibraryKind::PlaybackHistory,
            LibraryKind::Starred,
        ] {
            assert_eq!(LibraryKind::from_key(kind.key()), Some(kind));
        }
        assert_eq!(LibraryKind::from_key(" files "), None);
    }
}
