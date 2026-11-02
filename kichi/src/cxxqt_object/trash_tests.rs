use super::*;
use chrono::{TimeZone, Utc};
use kichi_core::{EntryKind, FileEntry, TrashEntry};
use std::collections::VecDeque;
use trash_actions::TrashActionKind;
use trash_mutation::{entries_to_restore, take_removed_entries};
use trash_view::sort_trash_entries;

fn entry(id: &str, deleted: Option<u32>) -> TrashEntry {
    TrashEntry {
        entry: FileEntry {
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
        },
        parent_id: String::new(),
        deleted_at: deleted.and_then(|day| Utc.with_ymd_and_hms(2026, 10, day, 0, 0, 0).single()),
    }
}

#[test]
fn newest_deleted_entries_come_first() {
    let mut entries = vec![
        entry("old", Some(1)),
        entry("none", None),
        entry("new", Some(20)),
    ];
    sort_trash_entries(&mut entries);
    let order: Vec<&str> = entries.iter().map(|item| item.entry.id.as_str()).collect();
    assert_eq!(order, vec!["new", "old", "none"]);
}

#[test]
fn progress_reports_completed_items_before_the_next_chunk() {
    assert_eq!(
        TrashActionKind::Restore.progress(0, 250),
        "正在还原……已处理 0/250 项"
    );
    assert_eq!(
        TrashActionKind::Restore.progress(100, 250),
        "正在还原……已处理 100/250 项"
    );
    assert_eq!(
        TrashActionKind::Delete.progress(0, 250),
        "正在彻底删除……已处理 0/250 项"
    );
    assert_eq!(
        TrashActionKind::Delete.progress(100, 250),
        "正在彻底删除……已处理 100/250 项"
    );
    assert_eq!(TrashActionKind::Clear.progress(0, 250), "正在清空回收站……");
}

#[test]
fn labels_distinguish_restore_delete_and_clear() {
    assert_eq!(TrashActionKind::Restore.success(2), "已还原 2 项");
    assert_eq!(TrashActionKind::Delete.success(2), "已彻底删除 2 项");
    assert_eq!(TrashActionKind::Clear.success(2), "回收站已清空（共 2 项）");
}

#[test]
fn optimistic_delete_removes_selected_rows_before_the_request() {
    let mut entries = vec![
        entry("a", Some(3)),
        entry("b", Some(2)),
        entry("c", Some(1)),
    ];
    let removed = take_removed_entries(&mut entries, &["a".to_owned(), "c".to_owned()]);

    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.entry.id.as_str())
            .collect::<Vec<_>>(),
        vec!["b"]
    );
    assert_eq!(
        removed
            .iter()
            .map(|entry| entry.entry.id.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "c"]
    );
}

#[test]
fn failed_delete_keeps_finished_items_deleted_and_restores_the_rest() {
    let mut removed = vec![
        entry("a", Some(3)),
        entry("b", Some(2)),
        entry("c", Some(1)),
    ];
    let remaining = VecDeque::from(["c".to_owned()]);
    let restored = entries_to_restore(
        TrashActionKind::Delete,
        &mut removed,
        &remaining,
        &["b".to_owned()],
    );

    assert_eq!(
        restored
            .iter()
            .map(|entry| entry.entry.id.as_str())
            .collect::<Vec<_>>(),
        vec!["b", "c"]
    );
    assert!(removed.is_empty(), "已成功的条目不再保留回滚快照");
}

#[test]
fn failed_clear_restores_the_whole_snapshot() {
    let mut removed = vec![entry("a", Some(2)), entry("b", Some(1))];
    let restored = entries_to_restore(TrashActionKind::Clear, &mut removed, &VecDeque::new(), &[]);

    assert_eq!(
        restored
            .iter()
            .map(|entry| entry.entry.id.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "b"]
    );
    assert!(removed.is_empty());
}
