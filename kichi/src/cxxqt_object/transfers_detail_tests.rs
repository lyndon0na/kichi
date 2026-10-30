//! `transfers_detail.rs` 的单元测试：文件夹内联展开的选中状态与快照。
//!
//! 全部用本地 `Vec<Transfer>` 调 `*_in` 纯函数，不碰进程级记录表。

use std::path::PathBuf;

use cxx_qt_lib::QString;
use kichi_core::EntryKind;

use super::*;
use crate::cxxqt_object::transfers::{row_from, TransferJob, TransferState};
use crate::cxxqt_object::transfers_view::QVariantMap;
use crate::cxxqt_object::QVariantList;

fn job(id: &str, name: &str, kind: EntryKind, sub_dir: &[&str]) -> TransferJob {
    TransferJob {
        id: id.to_owned(),
        name: name.to_owned(),
        kind,
        size: 10,
        audit_flagged: false,
        sub_dir: sub_dir.iter().map(|part| (*part).to_owned()).collect(),
        dir: PathBuf::from("/tmp"),
    }
}

/// 文件夹批次：父行 + 若干子行（index 0..）。
fn folder_batch(children: &[&str]) -> Vec<Transfer> {
    let seed = job("folder-1", "剧集", EntryKind::Folder, &[]);
    let mut records = vec![row_from(7, 0, "source", &seed, 1_790_000_000, 1)];
    for (index, name) in children.iter().enumerate() {
        let seed = job(name, name, EntryKind::File, &["剧集"]);
        let mut child = row_from(7, index, "source", &seed, 1_790_000_000, 1);
        child.child = true;
        records.push(child);
    }
    records
}

fn text_of(map: &QVariantMap, key: &str) -> String {
    map.get(&QString::from(key))
        .and_then(|value| value.value::<QString>())
        .map(|value| value.to_string())
        .unwrap_or_default()
}

fn map_at(list: &QVariantList, index: usize) -> QVariantMap {
    list.at(index)
        .and_then(|value| value.value::<QVariantMap>())
        .expect("快照的每一行都是 map")
}

#[test]
fn selecting_a_folder_with_children_works() {
    let records = folder_batch(&["a.mkv", "b.mkv"]);
    let mut selected = None;
    assert!(select_folder_in(&mut selected, &records, "7:folder"));
    assert_eq!(selected, Some(7));

    let (detail, active) = detail_snapshot_in(&mut selected, &records);
    assert!(active);
    assert_eq!(detail.len(), 2);
    assert_eq!(text_of(&map_at(&detail, 0), "key"), "7:0");
    assert_eq!(text_of(&map_at(&detail, 1), "key"), "7:1");
}

#[test]
fn selecting_rejects_invalid_keys_and_empty_folders() {
    let mut records = folder_batch(&["a.mkv"]);
    let mut selected = Some(1);
    assert!(
        !select_folder_in(&mut selected, &records, "7:0"),
        "子文件的 key 不能当文件夹选中"
    );
    assert!(
        !select_folder_in(&mut selected, &records, "9:folder"),
        "不存在的批次不能选中"
    );
    assert_eq!(selected, Some(1), "无效 key 不改变原来的选中");

    // 没有子行的文件夹（排队 / 读目录失败）不能展开
    records.truncate(1);
    let mut empty = None;
    assert!(!select_folder_in(&mut empty, &records, "7:folder"));
    assert_eq!(empty, None);
}

#[test]
fn selecting_a_preparing_folder_with_children_is_rejected() {
    let mut records = folder_batch(&["a.mkv"]);
    records[0].state = TransferState::Preparing;
    let mut selected = None;
    assert!(
        !select_folder_in(&mut selected, &records, "7:folder"),
        "准备中不能展开"
    );
    assert_eq!(selected, None);
}

#[test]
fn detail_snapshot_clears_selection_when_batch_disappears() {
    let mut records = folder_batch(&["a.mkv"]);
    let mut selected = None;
    assert!(select_folder_in(&mut selected, &records, "7:folder"));

    records.clear();
    let (detail, active) = detail_snapshot_in(&mut selected, &records);
    assert!(!active, "批次被清空 / 淘汰后展开自动收起");
    assert!(detail.is_empty());
    assert_eq!(selected, None, "失效的选中必须被清掉");
}

#[test]
fn detail_snapshot_clears_selection_when_children_are_gone() {
    let mut records = folder_batch(&["a.mkv"]);
    let mut selected = None;
    assert!(select_folder_in(&mut selected, &records, "7:folder"));

    // 子行全部被重试移走，父行还在（父行已结束，下一次刷新会被一起删）
    records.truncate(1);
    let (detail, active) = detail_snapshot_in(&mut selected, &records);
    assert!(!active);
    assert!(detail.is_empty());
    assert_eq!(selected, None);
}

#[test]
fn no_selection_means_no_detail() {
    let records = folder_batch(&["a.mkv"]);
    let mut selected = None;
    let (detail, active) = detail_snapshot_in(&mut selected, &records);
    assert!(!active);
    assert!(detail.is_empty());
}

#[test]
fn clear_selection_reports_whether_anything_changed() {
    let mut selected = Some(3);
    assert!(clear_selection_in(&mut selected));
    assert_eq!(selected, None);
    assert!(!clear_selection_in(&mut selected), "已经清了就返回 false");
}

#[test]
fn detail_snapshot_stays_valid_while_selected_folder_is_active() {
    let mut records = folder_batch(&["a.mkv", "b.mkv"]);
    records[0].state = TransferState::Running;
    records[1].state = TransferState::Done;
    let mut selected = None;
    assert!(select_folder_in(&mut selected, &records, "7:folder"));

    let (detail, active) = detail_snapshot_in(&mut selected, &records);
    assert!(active, "下载进行中仍可展开查看子行");
    assert_eq!(detail.len(), 2, "已完成的子行也一直显示");
}
