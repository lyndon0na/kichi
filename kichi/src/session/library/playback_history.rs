//! 播放历史查询：读取全部 `TYPE_PLAY` 事件，补齐必要文件信息后生成公共表格行。

use std::collections::HashSet;

use kichi_core::events::PLAY_HISTORY_LIMIT;
use kichi_core::{FileEntry, LibraryEntry, PlaybackHistoryEntry};

use super::super::{Failure, Session};

/// 与续播查找共用 20 页上限，避免异常分页把页面加载拖成无限扫描。
const MAX_HISTORY_PAGES: usize = 20;

pub(super) fn load(session: &Session) -> Result<Vec<LibraryEntry>, Failure> {
    let client = session.drive()?;
    let events = collect_history_pages(|page_token| {
        client.list_play_history(page_token).map_err(Failure::from)
    })?;
    entries_from_history(events, |id| client.entry_detail(id).map_err(Failure::from))
}

fn collect_history_pages<F>(mut fetch: F) -> Result<Vec<PlaybackHistoryEntry>, Failure>
where
    F: FnMut(Option<&str>) -> Result<(Vec<PlaybackHistoryEntry>, Option<String>), Failure>,
{
    let mut events = Vec::new();
    let mut page_token: Option<String> = None;
    let mut seen_tokens = HashSet::new();
    for _ in 0..MAX_HISTORY_PAGES {
        let (mut page, next) = fetch(page_token.as_deref())?;
        events.append(&mut page);
        match next {
            Some(token) if seen_tokens.insert(token.clone()) => page_token = Some(token),
            Some(_) => {
                return Err(Failure::Other(
                    "播放历史分页出现循环，已停止继续读取".to_owned(),
                ));
            }
            None => return Ok(events),
        }
    }
    Err(Failure::Other(format!(
        "播放历史超过 {} 条，已停止继续读取",
        MAX_HISTORY_PAGES * PLAY_HISTORY_LIMIT as usize
    )))
}

fn entries_from_history<F>(
    mut events: Vec<PlaybackHistoryEntry>,
    mut fetch_entry: F,
) -> Result<Vec<LibraryEntry>, Failure>
where
    F: FnMut(&str) -> Result<FileEntry, Failure>,
{
    // 官方通常已倒序且每文件一条；这里仍做稳定排序与去重，避免分页边界
    // 或服务端短暂重复让同一视频出现两行。去重发生在补详情之前，省掉重复请求。
    events.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    let mut seen_files = HashSet::new();
    events.retain(|entry| seen_files.insert(entry.file_id.clone()));

    let mut entries = Vec::with_capacity(events.len());
    for event in events {
        let mut entry = match event.reference_resource {
            Some(entry) => entry,
            None => fetch_entry(&event.file_id)?,
        };
        // 事件里的 file_id 是历史身份；资源缺 id 或回包字段漂移时仍以它为准。
        entry.id = event.file_id;
        entries.push(LibraryEntry {
            entry,
            event_id: (!event.id.trim().is_empty()).then_some(event.id),
            occurred_at: event.updated_at,
            progress: event.progress,
            folder_path: None,
        });
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use kichi_core::{EntryKind, PlaybackProgress};

    use super::*;

    fn file(id: &str) -> FileEntry {
        FileEntry {
            id: id.to_owned(),
            name: format!("{id}.mkv"),
            kind: EntryKind::File,
            size: 1,
            created_at: None,
            modified_at: None,
            parent_id: String::new(),
            md5: None,
            audit_flagged: false,
            starred: false,
            media: true,
            video: true,
            thumbnail_url: String::new(),
        }
    }

    fn event(
        id: &str,
        file_id: &str,
        day: u32,
        resource: Option<FileEntry>,
    ) -> PlaybackHistoryEntry {
        PlaybackHistoryEntry {
            id: id.to_owned(),
            file_id: file_id.to_owned(),
            updated_at: Utc.with_ymd_and_hms(2026, 10, day, 8, 0, 0).single(),
            progress: Some(PlaybackProgress {
                seconds: day as u64,
                duration: 100,
            }),
            reference_resource: resource,
        }
    }

    #[test]
    fn sorts_newest_first_and_dedupes_before_fetching_details() {
        let events = vec![
            event("e-old", "f1", 10, None),
            event("e2", "f2", 30, None),
            event("e-new", "f1", 20, None),
            event("e3", "f3", 15, Some(file("f3"))),
        ];
        let mut requested = Vec::new();
        let entries = entries_from_history(events, |id| {
            requested.push(id.to_owned());
            Ok(file(id))
        })
        .expect("历史应可生成");

        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.entry.id.as_str())
                .collect::<Vec<_>>(),
            ["f2", "f1", "f3"]
        );
        assert_eq!(entries[1].event_id.as_deref(), Some("e-new"));
        assert_eq!(
            requested,
            ["f2", "f1"],
            "重复文件去重后只补两次详情，完整资源不请求"
        );
    }

    #[test]
    fn complete_resource_generates_row_without_detail_request() {
        let entries = entries_from_history(vec![event("e1", "f1", 20, Some(file("f1")))], |_| {
            panic!("完整 reference_resource 不应再请求详情")
        })
        .expect("历史应可生成");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].entry.name, "f1.mkv");
        assert_eq!(
            entries[0].progress.map(|progress| progress.seconds),
            Some(20)
        );
        assert!(entries[0].occurred_at.is_some());
    }

    #[test]
    fn missing_resource_uses_event_id_for_detail_result() {
        let entries = entries_from_history(vec![event("e1", "event-file", 20, None)], |id| {
            let mut entry = file(id);
            entry.id.clear();
            Ok(entry)
        })
        .expect("历史应可生成");
        assert_eq!(entries[0].entry.id, "event-file");
    }

    #[test]
    fn pagination_collects_all_pages_and_rejects_token_cycles() {
        let first = event("e1", "f1", 20, Some(file("f1")));
        let second = event("e2", "f2", 19, Some(file("f2")));
        let events = collect_history_pages(|token| match token {
            None => Ok((vec![first.clone()], Some("next".to_owned()))),
            Some("next") => Ok((vec![second.clone()], None)),
            Some(other) => panic!("意外分页 token：{other}"),
        })
        .expect("两页历史应完整读取");
        assert_eq!(events.len(), 2);

        let cycle = collect_history_pages(|_| Ok((Vec::new(), Some("same-token".to_owned()))));
        assert!(cycle.is_err(), "重复 token 必须报错而不是无限翻页");
    }
}
