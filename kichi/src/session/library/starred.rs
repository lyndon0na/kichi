//! “已加星标”查询：索引就绪时读本地 `starred = 1 AND trashed = 0`，
//! 否则回退服务端 `system_tag=STAR` 筛选；两条路径都在本模块排序与去重。
//!
//! 星标是 PikPak 的系统标签，列表项上显式 `starred` 不可靠；`FileEntry`
//! 已经把显式字段与 `tags[].name == "STAR"` 统一成 `FileEntry::starred`。
//! 请求仍按它做本地校验，但不再遍历账号里的非星标文件。
//! 本模块只负责读取与排序；星标写请求走文件页共用的写操作入口。

use std::collections::HashSet;

use kichi_core::{FileEntry, LibraryEntry};

use super::super::{Failure, Session};
use super::LibraryLoad;

/// 索引来源的一次读取上限；云端的页数本身也没有硬上限。
const INDEX_STARRED_LIMIT: usize = 10_000;

/// 资料库「已加星标」：索引就绪时 0 网络，否则回退现有云端查询。
pub(super) fn load(session: &Session) -> Result<LibraryLoad, Failure> {
    if !session.local_index_ready() {
        let status = if session.local_index_enabled() {
            "本地索引尚未就绪，已从网盘读取"
        } else {
            "本地索引已关闭，已从网盘读取"
        };
        return Ok(LibraryLoad::new(load_cloud(session)?, status.to_owned()));
    }
    match load_index(session) {
        Ok(entries) => {
            let built = session
                .index_built_at()
                .map(format_built_time)
                .unwrap_or_default();
            let mut status = if built.is_empty() {
                "来自本地索引，未请求网盘".to_owned()
            } else {
                format!("来自本地索引（更新于 {built}），未请求网盘")
            };
            if entries.len() >= INDEX_STARRED_LIMIT {
                status.push_str(&format!("；仅显示前 {INDEX_STARRED_LIMIT} 项"));
            }
            Ok(LibraryLoad::new(entries, status))
        }
        Err(err) => {
            let reason = err.to_string();
            session.record_tree_index_error(format!("本地索引读取星标失败：{reason}"));
            Ok(LibraryLoad::new(
                load_cloud(session)?,
                "本地索引读取失败，已回退网盘读取".to_owned(),
            ))
        }
    }
}

fn load_index(session: &Session) -> Result<Vec<LibraryEntry>, Failure> {
    let index = session
        .tree_index()
        .ok_or_else(|| Failure::Other("本地索引未打开".to_owned()))?;
    let entries = index
        .starred(INDEX_STARRED_LIMIT)
        .map_err(|err| Failure::Other(format!("本地索引读取失败：{err}")))?;
    let mut result = starred_entries(entries);
    attach_folder_paths(session, &mut result);
    Ok(result)
}

/// 用索引里的父目录缓存补上「所在文件夹」；缺链的条目保持为空。
fn attach_folder_paths(session: &Session, entries: &mut [LibraryEntry]) {
    let Some(index) = session.tree_index() else {
        return;
    };
    let parents: Vec<String> = entries
        .iter()
        .map(|entry| entry.entry.parent_id.trim().to_owned())
        .filter(|parent| !parent.is_empty())
        .collect();
    if parents.is_empty() {
        return;
    }
    match index.folder_paths(&parents) {
        Ok(paths) => {
            for entry in entries {
                entry.folder_path = paths.get(entry.entry.parent_id.trim()).cloned();
            }
        }
        Err(err) => {
            session.record_tree_index_error(format!("本地索引解析已加星标文件夹路径失败：{err}"))
        }
    }
}

fn load_cloud(session: &Session) -> Result<Vec<LibraryEntry>, Failure> {
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

fn format_built_time(seconds: i64) -> String {
    match chrono::DateTime::from_timestamp(seconds, 0) {
        Some(time) => time
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M")
            .to_string(),
        None => String::new(),
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
