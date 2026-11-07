//! 文件夹连播队列：只保存元数据，不保存任何直链。
//!
//! 队列在开播时从目录快照生成，之后只读内存。上一集 / 下一集 / 播放列表
//! 都不会为了列表项去请求文件详情；直链只在真正要播某一集时取一条。

use crate::types::{EntryKind, FileEntry};

/// 队列里的一项（QML 只拿 id 与名字，永远拿不到 URL）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistItem {
    pub id: String,
    pub name: String,
    pub video: bool,
}

/// 一次播放的队列快照。
///
/// `entries` 保留目录里的完整条目（含字幕文件），供下一集的字幕计划复用；
/// `order` 是可播放条目在 `entries` 里的下标，顺序与目录列表一致。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Playlist {
    entries: Vec<FileEntry>,
    order: Vec<usize>,
    index: Option<usize>,
}

impl Playlist {
    /// 从目录快照生成队列，并定位当前条目。
    ///
    /// 当前条目不在快照里、或不是可播放文件时，`index` 为 `None`；
    /// 调用方应退回单集队列。
    pub fn from_entries(entries: Vec<FileEntry>, current_id: &str) -> Self {
        let order: Vec<usize> = entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| is_playable(entry))
            .map(|(index, _)| index)
            .collect();
        let index = order
            .iter()
            .position(|&entry_index| entries[entry_index].id == current_id);
        Self {
            entries,
            order,
            index,
        }
    }

    /// 只有当前这一集的队列：目录还没列出来、或当前文件不在目录里时使用。
    pub fn single(entry: FileEntry) -> Self {
        let playable = is_playable(&entry);
        Self {
            entries: vec![entry],
            order: if playable { vec![0] } else { Vec::new() },
            index: playable.then_some(0),
        }
    }

    /// 队列里没有可播放条目。
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// 可播放条目数（不含文件夹与非媒体文件）。
    pub fn len(&self) -> usize {
        self.order.len()
    }

    /// 当前条目在可播放顺序里的位置（0 起）。
    pub fn index(&self) -> Option<usize> {
        self.index
    }

    /// 完整目录快照：下一集的字幕计划直接从这里取候选。
    pub fn entries(&self) -> &[FileEntry] {
        &self.entries
    }

    pub fn current(&self) -> Option<&FileEntry> {
        self.index
            .and_then(|index| self.order.get(index))
            .map(|&entry_index| &self.entries[entry_index])
    }

    pub fn current_id(&self) -> Option<&str> {
        self.current().map(|entry| entry.id.as_str())
    }

    pub fn next(&self) -> Option<&FileEntry> {
        self.index
            .and_then(|index| self.order.get(index + 1))
            .map(|&entry_index| &self.entries[entry_index])
    }

    pub fn previous(&self) -> Option<&FileEntry> {
        self.index
            .and_then(|index| index.checked_sub(1))
            .and_then(|index| self.order.get(index))
            .map(|&entry_index| &self.entries[entry_index])
    }

    pub fn has_next(&self) -> bool {
        self.next().is_some()
    }

    pub fn has_previous(&self) -> bool {
        self.previous().is_some()
    }

    /// 按文件 id 定位；返回是否找到。
    pub fn select(&mut self, id: &str) -> bool {
        let Some(index) = self
            .order
            .iter()
            .position(|&entry_index| self.entries[entry_index].id == id)
        else {
            return false;
        };
        self.index = Some(index);
        true
    }

    /// 按队列位置（QML 播放列表的行号）定位；返回是否找到。
    pub fn select_index(&mut self, index: usize) -> bool {
        if index >= self.order.len() {
            return false;
        }
        self.index = Some(index);
        true
    }

    /// QML 播放列表模型：只有 id / 名字 / 是否视频。
    pub fn items(&self) -> Vec<PlaylistItem> {
        self.order
            .iter()
            .map(|&entry_index| {
                let entry = &self.entries[entry_index];
                PlaylistItem {
                    id: entry.id.clone(),
                    name: entry.name.clone(),
                    video: entry.video,
                }
            })
            .collect()
    }
}

/// 能进连播队列的条目：普通文件且官方归类为媒体（视频 / 音频）。
///
/// 文件列表不带 `kind` 过滤时官方分类为空，`FileEntry.media` 已按扩展名兜底。
pub fn is_playable(entry: &FileEntry) -> bool {
    entry.kind == EntryKind::File && entry.media
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, name: &str, kind: EntryKind, media: bool) -> FileEntry {
        FileEntry {
            id: id.to_owned(),
            name: name.to_owned(),
            kind,
            size: 1,
            created_at: None,
            modified_at: None,
            parent_id: String::new(),
            md5: None,
            audit_flagged: false,
            starred: false,
            media,
            video: media,
            thumbnail_url: String::new(),
        }
    }

    fn folder() -> Vec<FileEntry> {
        vec![
            entry("s1", "第1集.srt", EntryKind::File, false),
            entry("e1", "第1集.mkv", EntryKind::File, true),
            entry("dir", "花絮", EntryKind::Folder, false),
            entry("e2", "第2集.mkv", EntryKind::File, true),
            entry("e10", "第10集.mkv", EntryKind::File, true),
        ]
    }

    #[test]
    fn folders_and_subtitles_stay_out_of_the_queue() {
        let playlist = Playlist::from_entries(folder(), "e2");
        assert_eq!(playlist.len(), 3);
        assert_eq!(
            playlist
                .items()
                .into_iter()
                .map(|item| item.name)
                .collect::<Vec<_>>(),
            vec!["第1集.mkv", "第2集.mkv", "第10集.mkv"]
        );
        // 完整目录快照仍保留字幕文件，供字幕计划使用
        assert_eq!(playlist.entries().len(), 5);
    }

    #[test]
    fn next_and_previous_follow_the_queue_order() {
        let mut playlist = Playlist::from_entries(folder(), "e1");
        assert_eq!(playlist.index(), Some(0));
        assert!(!playlist.has_previous());
        assert_eq!(playlist.next().map(|entry| entry.id.as_str()), Some("e2"));
        assert!(playlist.select("e10"));
        assert_eq!(
            playlist.previous().map(|entry| entry.id.as_str()),
            Some("e2")
        );
        assert!(!playlist.has_next());
    }

    #[test]
    fn selecting_by_id_or_index_updates_the_current_entry() {
        let mut playlist = Playlist::from_entries(folder(), "e1");
        assert!(playlist.select_index(2));
        assert_eq!(playlist.current_id(), Some("e10"));
        assert!(!playlist.select_index(9));
        assert!(!playlist.select("missing"));
        assert_eq!(playlist.current_id(), Some("e10"));
    }

    #[test]
    fn a_missing_current_entry_leaves_the_queue_unpositioned() {
        let playlist = Playlist::from_entries(folder(), "missing");
        assert_eq!(playlist.len(), 3);
        assert_eq!(playlist.index(), None);
        assert_eq!(playlist.current(), None);
        assert_eq!(playlist.next(), None);
    }

    #[test]
    fn a_non_media_file_becomes_an_empty_queue() {
        let playlist = Playlist::single(entry("s1", "第1集.srt", EntryKind::File, false));
        assert!(playlist.is_empty());
        assert_eq!(playlist.current(), None);
    }
}
