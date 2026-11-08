//! 文件列表排序与条目映射的单元测试。

use super::*;
use crate::settings::SortField;
use chrono::{TimeZone, Utc};

fn entry(name: &str, kind: EntryKind, size: u64, day: u32) -> FileEntry {
    FileEntry {
        id: name.to_owned(),
        name: name.to_owned(),
        kind,
        size,
        created_at: None,
        modified_at: Utc.with_ymd_and_hms(2026, 1, day, 0, 0, 0).single(),
        parent_id: String::new(),
        md5: None,
        audit_flagged: false,
        starred: false,
        media: false,
        video: false,
        thumbnail_url: String::new(),
    }
}

fn names(entries: &[FileEntry]) -> Vec<&str> {
    entries.iter().map(|entry| entry.name.as_str()).collect()
}

#[test]
fn qml_map_exposes_media_flags_but_not_the_remote_url() {
    let mut entry = entry("cover.jpg", EntryKind::File, 10, 1);
    entry.video = true;
    entry.thumbnail_url = "https://thumb.example/secret".to_owned();
    let map = entry_map(&entry);

    assert_eq!(
        map.get(&QString::from("hasThumbnail"))
            .and_then(|value| value.value::<bool>()),
        Some(true)
    );
    assert_eq!(
        map.get(&QString::from("video"))
            .and_then(|value| value.value::<bool>()),
        Some(true)
    );
    assert!(map.get(&QString::from("thumbnailUrl")).is_none());
}

fn sample() -> Vec<FileEntry> {
    vec![
        entry("Beta.mkv", EntryKind::File, 300, 3),
        entry("alpha.mkv", EntryKind::File, 100, 1),
        entry("Zeta", EntryKind::Folder, 0, 20),
        entry("Aaa", EntryKind::Folder, 0, 10),
    ]
}

#[test]
fn folders_always_come_first() {
    for field in [SortField::Name, SortField::Size, SortField::Modified] {
        let mut entries = sample();
        sort_entries(&mut entries, field, false);
        let split = entries.partition_point(|e| e.kind == EntryKind::Folder);
        assert_eq!(split, 2, "{field:?} 降序时文件夹也必须在前");
        assert!(entries[..split].iter().all(|e| e.kind == EntryKind::Folder));
    }
}

#[test]
fn name_sort_is_case_insensitive() {
    let mut entries = sample();
    sort_entries(&mut entries, SortField::Name, true);
    assert_eq!(
        names(&entries),
        vec!["Aaa", "Zeta", "alpha.mkv", "Beta.mkv"]
    );
    sort_entries(&mut entries, SortField::Name, false);
    assert_eq!(
        names(&entries),
        vec!["Zeta", "Aaa", "Beta.mkv", "alpha.mkv"]
    );
}

#[test]
fn name_sort_puts_episode_numbers_in_value_order() {
    let mut entries = vec![
        entry("第10集.mkv", EntryKind::File, 1, 1),
        entry("第2集.mkv", EntryKind::File, 1, 1),
        entry("第1集.mkv", EntryKind::File, 1, 1),
    ];
    sort_entries(&mut entries, SortField::Name, true);
    assert_eq!(
        names(&entries),
        vec!["第1集.mkv", "第2集.mkv", "第10集.mkv"]
    );
}

#[test]
fn size_and_time_sort_apply_to_files_only_order() {
    let mut entries = sample();
    sort_entries(&mut entries, SortField::Size, true);
    assert_eq!(
        names(&entries),
        vec!["Aaa", "Zeta", "alpha.mkv", "Beta.mkv"]
    );
    sort_entries(&mut entries, SortField::Size, false);
    assert_eq!(
        names(&entries),
        vec!["Zeta", "Aaa", "Beta.mkv", "alpha.mkv"]
    );

    let mut entries = sample();
    sort_entries(&mut entries, SortField::Modified, true);
    assert_eq!(
        names(&entries),
        vec!["Aaa", "Zeta", "alpha.mkv", "Beta.mkv"]
    );
}

#[test]
fn equal_keys_fall_back_to_name_for_stable_output() {
    let mut entries = vec![
        entry("b", EntryKind::File, 5, 1),
        entry("a", EntryKind::File, 5, 1),
    ];
    sort_entries(&mut entries, SortField::Size, true);
    assert_eq!(names(&entries), vec!["a", "b"]);
}
