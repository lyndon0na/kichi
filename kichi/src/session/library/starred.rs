//! “已加星标”查询：服务端按 `system_tag=STAR` 筛选，再排序与去重。
//!
//! 星标是 PikPak 的系统标签，列表项上显式 `starred` 不可靠；`FileEntry`
//! 已经把显式字段与 `tags[].name == "STAR"` 统一成 `FileEntry::starred`。
//! 请求仍按它做本地校验，但不再遍历账号里的非星标文件。
//! 本模块只负责读取与排序；星标写请求走文件页共用的写操作入口。

use std::collections::HashSet;

use kichi_core::{FileEntry, LibraryEntry};

use super::super::{Failure, Session};

pub(super) fn load(session: &Session) -> Result<Vec<LibraryEntry>, Failure> {
    let client = session.drive()?;
    let mut entries = Vec::new();
    let mut page_token: Option<String> = None;
    let mut seen_tokens = HashSet::new();

    loop {
        let (page, next) = client
            .list_starred_page(page_token.as_deref())
            .map_err(Failure::from)?;
        entries.extend(page.into_iter().map(|hit| hit.entry));

        let Some(next) = next else {
            return Ok(starred_entries(entries));
        };
        // 分页 token 重复或原地打转时停止，避免把页面拖进无限请求。
        if !seen_tokens.insert(next.clone()) {
            return Err(Failure::Other(
                "已加星标分页出现循环，已停止继续读取".into(),
            ));
        }
        page_token = Some(next);
    }
}

fn starred_entries(entries: Vec<FileEntry>) -> Vec<LibraryEntry> {
    let mut starred: Vec<_> = entries
        .into_iter()
        .filter(|entry| entry.starred && !entry.id.trim().is_empty())
        .collect();

    starred.sort_by(|a, b| {
        b.modified_at
            .cmp(&a.modified_at)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.id.cmp(&b.id))
    });

    let mut seen_ids = HashSet::new();
    starred
        .into_iter()
        .filter(|entry| seen_ids.insert(entry.id.clone()))
        .map(|entry| {
            let occurred_at = entry.modified_at;
            let mut library_entry = LibraryEntry::new(entry);
            library_entry.occurred_at = occurred_at;
            library_entry
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use kichi_core::{EntryKind, FileEntry};

    use super::*;

    fn entry(id: &str, name: &str, starred: bool, day: Option<u32>) -> FileEntry {
        FileEntry {
            id: id.to_owned(),
            name: name.to_owned(),
            kind: EntryKind::File,
            size: 1,
            created_at: None,
            modified_at: day.and_then(|day| Utc.with_ymd_and_hms(2026, 10, day, 12, 0, 0).single()),
            parent_id: String::new(),
            md5: None,
            audit_flagged: false,
            starred,
            media: false,
            video: false,
            thumbnail_url: String::new(),
        }
    }

    #[test]
    fn filters_starred_and_sorts_by_modified_time_desc() {
        let entries = vec![
            entry("old", "old.txt", true, Some(20)),
            entry("plain", "plain.txt", false, Some(30)),
            entry("new", "new.txt", true, Some(28)),
            entry("unknown", "unknown.txt", true, None),
        ];

        let result = starred_entries(entries);
        let ids: Vec<_> = result.iter().map(|entry| entry.entry.id.as_str()).collect();
        assert_eq!(ids, ["new", "old", "unknown"]);
        assert_eq!(
            result[0].occurred_at, result[0].entry.modified_at,
            "星标页的时间列应显示更新时间"
        );
        assert!(result.iter().all(|entry| entry.event_id.is_none()));
        assert!(result.iter().all(|entry| entry.progress.is_none()));
    }

    #[test]
    fn duplicate_ids_keep_the_newest_starred_entry() {
        let entries = vec![
            entry("same", "older.txt", true, Some(10)),
            entry("same", "newer.txt", true, Some(20)),
            entry("same", "unstarred-newest.txt", false, Some(30)),
        ];

        let result = starred_entries(entries);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].entry.name, "newer.txt");
    }

    #[test]
    fn empty_or_missing_ids_are_ignored() {
        let entries = vec![
            entry("", "blank.txt", true, Some(20)),
            entry("   ", "spaces.txt", true, Some(21)),
            entry("valid", "valid.txt", true, Some(19)),
        ];

        let result = starred_entries(entries);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].entry.id, "valid");
    }
}
