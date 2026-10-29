use std::path::PathBuf;
use std::time::{Duration, Instant};

use cxx_qt_lib::QVariant;
use kichi_core::EntryKind;

use super::*;
use crate::cxxqt_object::transfers::{row_from, TransferJob};
use crate::cxxqt_object::transfers_view::QVariantMap;

fn job(id: &str, name: &str, kind: EntryKind, sub_dir: &[&str]) -> TransferJob {
    TransferJob {
        id: id.to_owned(),
        name: name.to_owned(),
        kind,
        size: 100,
        audit_flagged: false,
        sub_dir: sub_dir.iter().map(|part| (*part).to_owned()).collect(),
        dir: PathBuf::from("/tmp"),
    }
}

fn map_at(list: &QVariantList, index: isize) -> QVariantMap {
    list.0
        .get(index)
        .and_then(|value| value.value::<QVariantMap>())
        .expect("QVariantMap")
}

fn lane_at(list: &cxx_qt_lib::QList<QVariant>, index: isize) -> QVariantMap {
    list.get(index)
        .and_then(|value| value.value::<QVariantMap>())
        .expect("lane QVariantMap")
}

fn text_of(map: &QVariantMap, key: &str) -> String {
    map.get(&cxx_qt_lib::QString::from(key))
        .and_then(|value| value.value::<cxx_qt_lib::QString>())
        .map(|value| value.to_string())
        .unwrap_or_default()
}

fn int_of(map: &QVariantMap, key: &str) -> i32 {
    map.get(&cxx_qt_lib::QString::from(key))
        .and_then(|value| value.value::<i32>())
        .unwrap_or_default()
}

fn list_of(map: &QVariantMap, key: &str) -> cxx_qt_lib::QList<QVariant> {
    map.get(&cxx_qt_lib::QString::from(key))
        .and_then(|value| value.value::<cxx_qt_lib::QList<QVariant>>())
        .unwrap_or_default()
}

#[test]
fn independent_files_share_a_generic_summary() {
    let mut records = Vec::new();
    for (batch, name) in [(1, "a.mkv"), (2, "b.zip"), (3, "c.mp4")] {
        let mut row = row_from(
            batch,
            0,
            &format!("src-{batch}"),
            &job(&format!("id-{batch}"), name, EntryKind::File, &[]),
            1_790_000_000,
            1,
        );
        row.state = TransferState::Running;
        row.speed.begin(Instant::now());
        records.push(row);
    }

    let summary = snapshot_at(&records, Instant::now());
    let group = map_at(&summary, 0);
    assert_eq!(text_of(&group, "kind"), "files");
    assert_eq!(text_of(&group, "name"), "", "独立文件不能伪造文件夹标题");
    assert_eq!(int_of(&group, "runningCount"), 3);
    let lanes = list_of(&group, "lanes");
    assert_eq!(lanes.len(), 3, "每个独立文件各占一条摘要");
    assert_eq!(text_of(&lane_at(&lanes, 0), "name"), "a.mkv");
    assert_eq!(text_of(&lane_at(&lanes, 1), "name"), "b.zip");
}

#[test]
fn folder_summary_uses_folder_name_and_running_children() {
    let mut records = Vec::new();
    let mut folder = row_from(
        7,
        0,
        "folder-src",
        &job("folder-id", "周末短片", EntryKind::Folder, &[]),
        1_790_000_000,
        1,
    );
    folder.state = TransferState::Running;
    records.push(folder);
    for (index, name) in ["第一集.mkv", "第二集.mkv"].iter().enumerate() {
        let mut child = row_from(
            7,
            index,
            "folder-src",
            &job(
                &format!("file-{index}"),
                name,
                EntryKind::File,
                &["周末短片"],
            ),
            1_790_000_000,
            1,
        );
        child.child = true;
        child.state = if index == 0 {
            TransferState::Running
        } else {
            TransferState::Queued
        };
        child.speed.begin(Instant::now() - Duration::from_secs(1));
        records.push(child);
    }

    let summary = snapshot_at(&records, Instant::now());
    let group = map_at(&summary, 0);
    assert_eq!(text_of(&group, "kind"), "folder");
    assert_eq!(text_of(&group, "name"), "周末短片");
    assert_eq!(int_of(&group, "runningCount"), 1);
    assert_eq!(int_of(&group, "totalCount"), 2);
    let lanes = list_of(&group, "lanes");
    assert_eq!(lanes.len(), 1);
    assert_eq!(
        text_of(&lane_at(&lanes, 0), "name"),
        "第一集.mkv",
        "文件夹标题已经给出根目录，行内不再重复"
    );
}

#[test]
fn preparing_folder_has_no_lanes() {
    let mut folder = row_from(
        9,
        0,
        "folder-src",
        &job("folder-id", "待读目录", EntryKind::Folder, &[]),
        1_790_000_000,
        1,
    );
    folder.state = TransferState::Preparing;
    let summary = snapshot_at(&[folder], Instant::now());
    let group = map_at(&summary, 0);
    assert_eq!(text_of(&group, "state"), "preparing");
    assert!(list_of(&group, "lanes").is_empty());
}

#[test]
fn queued_only_has_no_active_summary() {
    let row = row_from(
        2,
        0,
        "src",
        &job("id", "queued.mkv", EntryKind::File, &[]),
        1_790_000_000,
        1,
    );
    assert!(snapshot_at(&[row], Instant::now()).is_empty());
}
