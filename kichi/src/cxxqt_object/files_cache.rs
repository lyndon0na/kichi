//! 文件页目录快照缓存。
//!
//! 目录 id → 最近一次条目快照。页面重新进入目录时先恢复快照，只有快照过期
//! 或显式刷新时才在后台校准；缓存设置上限，避免长时间浏览后无限增长。

use std::collections::HashMap;
use std::time::{Duration, Instant};

use kichi_core::FileEntry;

/// 目录快照的默认有效期。
pub(crate) const FILES_REFRESH_TTL: Duration = Duration::from_secs(30);
const MAX_CACHED_DIRS: usize = 32;

#[derive(Clone)]
struct CachedDirectory {
    entries: Vec<FileEntry>,
    /// `None` 表示条目仍可用于先显示，但下一次进入时必须后台校准。
    fetched_at: Option<Instant>,
}

#[derive(Default)]
pub(crate) struct FilesCache {
    directories: HashMap<String, CachedDirectory>,
}

impl FilesCache {
    pub(crate) fn snapshot(&self, dir_id: &str) -> Option<Vec<FileEntry>> {
        self.directories
            .get(dir_id)
            .map(|cache| cache.entries.clone())
    }

    pub(crate) fn contains(&self, dir_id: &str) -> bool {
        self.directories.contains_key(dir_id)
    }

    pub(crate) fn is_fresh(&self, dir_id: &str, now: Instant) -> bool {
        self.directories.get(dir_id).is_some_and(|cache| {
            cache.fetched_at.is_some_and(|fetched_at| {
                now.saturating_duration_since(fetched_at) < FILES_REFRESH_TTL
            })
        })
    }

    /// 保存一次成功读取的完整快照。
    pub(crate) fn store(&mut self, dir_id: String, entries: &[FileEntry], now: Instant) {
        if !self.directories.contains_key(&dir_id) && self.directories.len() >= MAX_CACHED_DIRS {
            let oldest = self
                .directories
                .iter()
                .min_by_key(|(_, cache)| cache.fetched_at)
                .map(|(id, _)| id.clone());
            if let Some(id) = oldest {
                self.directories.remove(&id);
            }
        }
        self.directories.insert(
            dir_id,
            CachedDirectory {
                entries: entries.to_vec(),
                fetched_at: Some(now),
            },
        );
    }

    /// 本地乐观修改后更新快照内容，并保留原来的 TTL 时刻。
    pub(crate) fn replace_entries(&mut self, dir_id: String, entries: &[FileEntry]) {
        if let Some(cache) = self.directories.get_mut(&dir_id) {
            cache.entries = entries.to_vec();
        } else {
            self.store(dir_id, entries, Instant::now());
        }
    }

    /// 星标写入失败时恢复目录快照里的对应字段。
    pub(crate) fn restore_starred(&mut self, dir_id: &str, changes: &[(String, bool)]) {
        let Some(cache) = self.directories.get_mut(dir_id) else {
            return;
        };
        for (id, starred) in changes {
            if let Some(entry) = cache.entries.iter_mut().find(|entry| entry.id == *id) {
                entry.starred = *starred;
            }
        }
    }

    /// 改名失败时只恢复这一条的名称，不动同目录其它并发动作。
    pub(crate) fn restore_name(&mut self, dir_id: &str, id: &str, name: &str) {
        let Some(cache) = self.directories.get_mut(dir_id) else {
            return;
        };
        if let Some(entry) = cache.entries.iter_mut().find(|entry| entry.id == id) {
            entry.name = name.to_owned();
        }
    }

    /// 新建文件夹失败时移除临时行。
    pub(crate) fn remove_entry(&mut self, dir_id: &str, id: &str) {
        let Some(cache) = self.directories.get_mut(dir_id) else {
            return;
        };
        cache.entries.retain(|entry| entry.id != id);
    }

    /// 移动 / 回收站失败时把条目按原位置放回缓存，保留其它动作造成的改动。
    pub(crate) fn restore_entries(&mut self, dir_id: &str, removed: &[(usize, FileEntry)]) {
        let Some(cache) = self.directories.get_mut(dir_id) else {
            return;
        };
        for (index, entry) in removed {
            if cache.entries.iter().any(|current| current.id == entry.id) {
                continue;
            }
            let at = (*index).min(cache.entries.len());
            cache.entries.insert(at, entry.clone());
        }
    }

    /// 请求失败后短暂退避，避免每次切回页面都立刻重试同一个失败目录。
    pub(crate) fn defer_refresh(&mut self, dir_id: &str, now: Instant) {
        if let Some(cache) = self.directories.get_mut(dir_id) {
            cache.fetched_at = Some(now);
        }
    }

    /// 保留可用于首屏显示的旧快照，但下一次进入必须重新校准。
    pub(crate) fn mark_stale(&mut self, dir_id: &str) {
        if let Some(cache) = self.directories.get_mut(dir_id) {
            cache.fetched_at = None;
        }
    }

    pub(crate) fn mark_all_stale(&mut self) {
        for cache in self.directories.values_mut() {
            cache.fetched_at = None;
        }
    }

    pub(crate) fn clear(&mut self) {
        self.directories.clear();
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.directories.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kichi_core::EntryKind;

    fn entry(id: &str) -> FileEntry {
        FileEntry {
            id: id.to_owned(),
            name: id.to_owned(),
            kind: EntryKind::File,
            size: 1,
            created_at: None,
            modified_at: None,
            parent_id: String::new(),
            md5: None,
            audit_flagged: false,
            starred: false,
            media: false,
            video: false,
            thumbnail_url: String::new(),
        }
    }

    #[test]
    fn restores_each_directory_snapshot_independently() {
        let mut cache = FilesCache::default();
        let now = Instant::now();
        cache.store("root".to_owned(), &[entry("root-file")], now);
        cache.store("dir-a".to_owned(), &[entry("a-file")], now);

        assert_eq!(
            cache
                .snapshot("dir-a")
                .and_then(|entries| entries.first().map(|entry| entry.id.clone())),
            Some("a-file".to_owned())
        );
        assert_eq!(
            cache
                .snapshot("root")
                .and_then(|entries| entries.first().map(|entry| entry.id.clone())),
            Some("root-file".to_owned())
        );
    }

    #[test]
    fn stale_snapshot_remains_available_but_is_not_fresh() {
        let mut cache = FilesCache::default();
        let now = Instant::now();
        cache.store("dir".to_owned(), &[entry("file")], now);
        assert!(cache.is_fresh("dir", now));

        cache.mark_stale("dir");
        assert!(cache.snapshot("dir").is_some());
        assert!(!cache.is_fresh("dir", now));
    }

    #[test]
    fn ttl_boundary_expires_at_thirty_seconds() {
        let mut cache = FilesCache::default();
        let now = Instant::now();
        cache.store("dir".to_owned(), &[entry("file")], now);

        assert!(cache.is_fresh("dir", now + Duration::from_secs(29)));
        assert!(!cache.is_fresh("dir", now + FILES_REFRESH_TTL));
    }

    #[test]
    fn cache_evicts_an_old_directory_at_the_cap() {
        let mut cache = FilesCache::default();
        let now = Instant::now();
        for index in 0..=MAX_CACHED_DIRS {
            let at = now + Duration::from_millis(index as u64);
            cache.store(format!("dir-{index}"), &[], at);
        }

        assert_eq!(cache.len(), MAX_CACHED_DIRS);
        assert!(!cache.contains("dir-0"));
        assert!(cache.contains(&format!("dir-{MAX_CACHED_DIRS}")));
    }
}
