//! “最近保存”查询：上传 / 保存事件 + 文件资源，按更新时间新到旧。

use std::collections::{HashMap, HashSet};

use kichi_core::events::RECENT_SAVED_LIMIT;
use kichi_core::{FileEntry, LibraryEntry, RecentSavedEvent};

use super::super::{Failure, Session};

const TARGET_ROWS: usize = RECENT_SAVED_LIMIT as usize;
const MAX_EVENT_PAGES: usize = 5;

pub(super) fn load(session: &Session) -> Result<Vec<LibraryEntry>, Failure> {
    let client = session.drive()?;
    let mut deduped = HashMap::<String, RecentSavedEvent>::new();
    let mut page_token: Option<String> = None;
    let mut seen_tokens = HashSet::new();

    for _ in 0..MAX_EVENT_PAGES {
        let (events, next) = client
            .list_recent_saved(page_token.as_deref())
            .map_err(Failure::from)?;
        for event in events {
            merge_event(&mut deduped, event);
        }
        if deduped.len() >= TARGET_ROWS {
            break;
        }
        let Some(token) = next else {
            break;
        };
        if !seen_tokens.insert(token.clone()) {
            return Err(Failure::Other(
                "最近保存事件分页出现循环，已停止继续读取".to_owned(),
            ));
        }
        page_token = Some(token);
    }

    let events = ordered_events(deduped);
    assemble(events, |id| client.file_entry(id).map_err(Failure::from))
}

fn merge_event(events: &mut HashMap<String, RecentSavedEvent>, candidate: RecentSavedEvent) {
    match events.entry(candidate.file_id.clone()) {
        std::collections::hash_map::Entry::Occupied(mut current) => {
            if newer(&candidate, current.get()) {
                current.insert(candidate);
            }
        }
        std::collections::hash_map::Entry::Vacant(slot) => {
            slot.insert(candidate);
        }
    }
}

fn newer(candidate: &RecentSavedEvent, current: &RecentSavedEvent) -> bool {
    candidate.occurred_at > current.occurred_at
        || (candidate.occurred_at == current.occurred_at
            && candidate.resource.is_some()
            && current.resource.is_none())
}

fn ordered_events(events: HashMap<String, RecentSavedEvent>) -> Vec<RecentSavedEvent> {
    let mut events: Vec<_> = events.into_values().collect();
    events.sort_by(|left, right| {
        right
            .occurred_at
            .cmp(&left.occurred_at)
            .then_with(|| left.file_id.cmp(&right.file_id))
    });
    events.truncate(TARGET_ROWS);
    events
}

fn assemble(
    events: Vec<RecentSavedEvent>,
    mut load_entry: impl FnMut(&str) -> Result<FileEntry, Failure>,
) -> Result<Vec<LibraryEntry>, Failure> {
    events
        .into_iter()
        .map(|event| {
            let entry = match event.resource {
                Some(entry) => entry,
                None => load_entry(&event.file_id)?,
            };
            Ok(LibraryEntry {
                entry,
                event_id: event.event_id,
                occurred_at: event.occurred_at,
                progress: None,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::*;
    use kichi_core::EntryKind;

    fn time(hour: u32) -> Option<chrono::DateTime<Utc>> {
        Utc.with_ymd_and_hms(2026, 10, 31, hour, 0, 0).single()
    }

    fn entry(id: &str, kind: EntryKind) -> FileEntry {
        FileEntry {
            id: id.to_owned(),
            name: format!("{id}.mkv"),
            kind,
            size: 42,
            created_at: None,
            modified_at: None,
            parent_id: "parent".to_owned(),
            md5: None,
            audit_flagged: false,
            starred: false,
            media: true,
            video: true,
            thumbnail_url: String::new(),
        }
    }

    fn event(id: &str, event_id: &str, hour: u32, resource: Option<FileEntry>) -> RecentSavedEvent {
        RecentSavedEvent {
            event_id: Some(event_id.to_owned()),
            file_id: id.to_owned(),
            occurred_at: time(hour),
            resource,
        }
    }

    #[test]
    fn newer_event_wins_and_rows_are_sorted_newest_first() {
        let mut events = HashMap::new();
        merge_event(
            &mut events,
            event("a", "old", 8, Some(entry("a", EntryKind::File))),
        );
        merge_event(
            &mut events,
            event("a", "new", 10, Some(entry("a", EntryKind::File))),
        );
        merge_event(
            &mut events,
            event("b", "middle", 9, Some(entry("b", EntryKind::Folder))),
        );

        let ordered = ordered_events(events);
        assert_eq!(
            ordered
                .iter()
                .map(|event| event.event_id.as_deref())
                .collect::<Vec<_>>(),
            vec![Some("new"), Some("middle")]
        );
    }

    #[test]
    fn complete_resource_needs_no_detail_request() {
        let rows = assemble(
            vec![event("a", "event", 10, Some(entry("a", EntryKind::File)))],
            |_| panic!("完整资源不应请求详情"),
        )
        .expect("应转换成功");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].entry.name, "a.mkv");
        assert_eq!(rows[0].event_id.as_deref(), Some("event"));
        assert_eq!(rows[0].occurred_at, time(10));
        assert!(rows[0].progress.is_none());
    }

    #[test]
    fn incomplete_resource_fetches_detail_once_and_keeps_event_time() {
        let mut calls = Vec::new();
        let rows = assemble(vec![event("a", "event", 10, None)], |id| {
            calls.push(id.to_owned());
            Ok(entry(id, EntryKind::Folder))
        })
        .expect("应补详情成功");

        assert_eq!(calls, vec!["a"]);
        assert_eq!(rows[0].entry.kind, EntryKind::Folder);
        assert_eq!(rows[0].occurred_at, time(10));
    }
}
