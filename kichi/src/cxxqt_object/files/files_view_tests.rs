//! 文件列表排序与条目映射的单元测试。

use super::*;
use crate::cxxqt_object::files::Crumb;
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

fn crumb(id: &str, name: &str) -> Crumb {
    Crumb {
        id: id.to_owned(),
        name: name.to_owned(),
    }
}

fn list_values(list: &QStringList) -> Vec<String> {
    list.iter().map(|value| value.to_string()).collect()
}

#[test]
fn breadcrumb_ids_mirror_names_at_root() {
    let (names, ids) = breadcrumb_lists(&[Crumb::root()]);

    assert_eq!(list_values(&names), vec!["全部文件"]);
    assert_eq!(list_values(&ids), vec![""], "根目录 id 必须是空串");
    assert_eq!(names.len(), ids.len(), "两条面包屑必须一一对应");
}

#[test]
fn breadcrumb_ids_mirror_names_in_deep_directories() {
    let crumbs = [Crumb::root(), crumb("id-a", "A"), crumb("id-b", "B")];
    let (names, ids) = breadcrumb_lists(&crumbs);

    assert_eq!(list_values(&names), vec!["全部文件", "A", "B"]);
    assert_eq!(list_values(&ids), vec!["", "id-a", "id-b"]);
    assert_eq!(names.len(), ids.len());
}

#[test]
fn breadcrumb_ids_stay_in_sync_across_search_round_trip() {
    // 搜索只切换列表 model，不改目录栈；退出搜索后两条面包屑仍要指向同一组目录。
    let crumbs = [Crumb::root(), crumb("id-a", "A")];
    let (before_names, before_ids) = breadcrumb_lists(&crumbs);
    let (after_names, after_ids) = breadcrumb_lists(&crumbs);

    assert_eq!(list_values(&before_names), list_values(&after_names));
    assert_eq!(list_values(&before_ids), list_values(&after_ids));
    assert_eq!(list_values(&after_ids), vec!["", "id-a"]);
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

#[test]
fn qml_map_marks_image_files_only() {
    let image = entry("cover.JPG", EntryKind::File, 10, 1);
    assert_eq!(
        entry_map(&image)
            .get(&QString::from("image"))
            .and_then(|value| value.value::<bool>()),
        Some(true)
    );
    for not_image in [
        entry("movie.mkv", EntryKind::File, 10, 1),
        entry("album.png", EntryKind::Folder, 0, 1),
    ] {
        assert_eq!(
            entry_map(&not_image)
                .get(&QString::from("image"))
                .and_then(|value| value.value::<bool>()),
            Some(false)
        );
    }
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

#[test]
fn filtered_list_hides_non_matching_rows_and_keeps_the_rest_in_order() {
    let entries = vec![
        entry("合集", EntryKind::Folder, 0, 1),
        entry("电影.mkv", EntryKind::File, 20, 1),
        entry("笔记.txt", EntryKind::File, 1, 1),
    ];
    let pending = std::collections::HashSet::new();
    let now = Utc
        .with_ymd_and_hms(2026, 1, 3, 0, 0, 0)
        .unwrap()
        .timestamp();

    assert_eq!(
        list_filtered(
            &entries,
            &pending,
            &kichi_core::SearchFilter::default(),
            now
        )
        .len(),
        entries.len(),
        "没有筛选时和完整列表一致"
    );

    let video = kichi_core::SearchFilter {
        types: vec![kichi_core::TypeFilter::Video],
        ..Default::default()
    };
    assert_eq!(
        list_filtered(&entries, &pending, &video, now).len(),
        1,
        "只看视频：文件夹与文档都被藏起来"
    );

    let small = kichi_core::SearchFilter {
        sizes: vec![kichi_core::SizeFilter::Lt10MiB],
        ..Default::default()
    };
    assert_eq!(
        list_filtered(&entries, &pending, &small, now).len(),
        2,
        "按大小筛时文件夹没有大小，不显示"
    );
}
