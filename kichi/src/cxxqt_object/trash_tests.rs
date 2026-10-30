use super::*;
use chrono::{TimeZone, Utc};
use kichi_core::{EntryKind, FileEntry, TrashEntry};
use trash_actions::TrashActionKind;
use trash_view::sort_trash_entries;

fn entry(id: &str, deleted: Option<u32>) -> TrashEntry {
    TrashEntry {
        entry: FileEntry {
            id: id.to_owned(),
            name: id.to_owned(),
            kind: EntryKind::File,
            size: 1,
            modified_at: None,
            md5: None,
            audit_flagged: false,
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
        TrashActionKind::Delete.progress(0, 250),
        "正在彻底删除……已处理 0/250 项"
    );
    assert_eq!(
        TrashActionKind::Delete.progress(100, 250),
        "正在彻底删除……已处理 100/250 项"
    );
}

#[test]
fn labels_distinguish_restore_delete_and_clear() {
    assert_eq!(TrashActionKind::Restore.success(2), "已还原 2 项");
    assert_eq!(TrashActionKind::Delete.success(2), "已彻底删除 2 项");
    assert!(TrashActionKind::Clear.success(2).contains("清空"));
}
