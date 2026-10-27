//! `transfers.rs` 的单元测试：记录表的状态迁移、排序、上限与显示字段。
//!
//! 拆出来只为守住行数软上限（实现本身约 350 行）。

use std::path::{Path, PathBuf};

use cxx_qt_lib::QString;
use kichi_core::EntryKind;

use super::*;
use crate::cxxqt_object::transfers_view::{
    build_snapshot, notice_map, ordered, percent_of, record_map, QVariantMap,
};

/// 一条记录的种子（名字就是裸名字，单文件层级）。
fn job(name: &str) -> TransferJob {
    TransferJob {
        id: format!("id-{name}"),
        name: name.to_owned(),
        kind: EntryKind::File,
        size: 0,
        audit_flagged: false,
        sub_dir: Vec::new(),
        dir: PathBuf::from("/tmp"),
    }
}

fn add_batch(records: &mut Vec<Transfer>, seq: u64, names: &[&str]) {
    for (index, name) in names.iter().enumerate() {
        records.push(row_from(seq, index, "source", &job(name)));
    }
}

fn set(records: &mut [Transfer], batch: u64, index: usize, state: TransferState) {
    if let Some(record) = find(records, batch, index) {
        record.state = state;
    }
}

fn names(records: &[Transfer]) -> Vec<String> {
    records.iter().map(Transfer::display_name).collect()
}

fn text_of(map: &QVariantMap, key: &str) -> String {
    map.get(&QString::from(key))
        .and_then(|value| value.value::<QString>())
        .map(|value| value.to_string())
        .unwrap_or_default()
}

#[test]
fn active_rows_come_first_in_batch_order() {
    let mut records = Vec::new();
    add_batch(&mut records, 1, &["a", "b", "c"]);
    set(&mut records, 1, 0, TransferState::Done);
    set(&mut records, 1, 2, TransferState::Running);
    let ordered = ordered(&records);
    let names: Vec<String> = ordered.iter().map(|record| record.display_name()).collect();
    assert_eq!(
        names,
        vec!["b", "c", "a"],
        "进行中在前（批内顺序），已结束在后"
    );
}

#[test]
fn finished_rows_are_newest_first() {
    let mut records = Vec::new();
    add_batch(&mut records, 1, &["old"]);
    add_batch(&mut records, 2, &["new"]);
    cancel_active_in(&mut records, None, "");
    let ordered = ordered(&records);
    let names: Vec<String> = ordered.iter().map(|record| record.display_name()).collect();
    assert_eq!(names, vec!["new", "old"]);
}

#[test]
fn progress_only_applies_to_running() {
    let mut records = Vec::new();
    add_batch(&mut records, 1, &["a"]);
    progress_in(&mut records, 1, 0, 500, Some(1000));
    assert_eq!(records[0].bytes, 0, "排队中的条目忽略进度");

    begin_in(&mut records, 1, 0);
    progress_in(&mut records, 1, 0, 500, Some(1000));
    assert_eq!(records[0].bytes, 500);

    cancel_active_in(&mut records, None, "");
    progress_in(&mut records, 1, 0, 900, Some(1000));
    assert_eq!(records[0].state, TransferState::Cancelled);
    assert_eq!(records[0].bytes, 500, "取消后迟到的进度不能复活记录");
}

#[test]
fn note_only_touches_active_rows() {
    let mut records = Vec::new();
    add_batch(&mut records, 1, &["a", "b"]);
    note_in(&mut records, 1, 0, "排队中不该有提示");
    assert_eq!(records[0].detail, "", "排队中的行不接受 note");

    begin_in(&mut records, 1, 0);
    note_in(&mut records, 1, 0, "已从 45.0 MB 处继续");
    assert_eq!(records[0].detail, "已从 45.0 MB 处继续");

    finish_ok_in(&mut records, 1, 0, Path::new("/tmp/a.mkv"));
    note_in(&mut records, 1, 0, "迟到的提示");
    assert_eq!(records[0].detail, "", "完成行不显示旧提示");
}

#[test]
fn finishing_records_path_instead_of_detail() {
    let mut records = Vec::new();
    add_batch(&mut records, 1, &["a", "b"]);
    begin_in(&mut records, 1, 0);
    progress_in(&mut records, 1, 0, 1024, Some(1024));
    finish_ok_in(&mut records, 1, 0, Path::new("/home/me/Downloads/a.mkv"));
    assert_eq!(records[0].state, TransferState::Done);
    assert_eq!(
        records[0].path,
        Some(PathBuf::from("/home/me/Downloads/a.mkv")),
        "落盘路径存成 PathBuf"
    );
    assert!(
        records[0].detail.is_empty(),
        "完成行不再往 detail 里拼字符串"
    );

    begin_in(&mut records, 1, 1);
    finish_err_in(&mut records, 1, 1, "下载中断：连接被重置");
    assert_eq!(records[1].state, TransferState::Failed);
    assert_eq!(records[1].detail, "下载中断：连接被重置");
    assert_eq!(records[1].path, None, "失败行没有落盘路径");
}

#[test]
fn cancel_marks_every_active_row() {
    let mut records = Vec::new();
    add_batch(&mut records, 1, &["a", "b", "c"]);
    set(&mut records, 1, 0, TransferState::Done);
    begin_in(&mut records, 1, 1);
    cancel_active_in(&mut records, None, "");
    assert_eq!(records[0].state, TransferState::Done, "已完成的不动");
    assert_eq!(records[1].state, TransferState::Cancelled);
    assert_eq!(records[2].state, TransferState::Cancelled);
}

#[test]
fn clear_finished_keeps_active() {
    let mut records = Vec::new();
    add_batch(&mut records, 1, &["a", "b"]);
    set(&mut records, 1, 0, TransferState::Done);
    clear_finished_in(&mut records);
    assert_eq!(names(&records), vec!["b"]);
    assert_eq!(records[0].state, TransferState::Queued);
}

#[test]
fn trim_drops_oldest_finished_but_keeps_active() {
    let mut records = Vec::new();
    for index in 0..MAX_RECORDS {
        let mut record = row_from(1, index, "", &job(&format!("done-{index}")));
        record.state = TransferState::Done;
        records.push(record);
    }
    for index in 0..2 {
        let mut record = row_from(1, MAX_RECORDS + index, "", &job(&format!("active-{index}")));
        record.state = TransferState::Running;
        records.push(record);
    }
    trim(&mut records);

    assert_eq!(records.len(), MAX_RECORDS);
    assert_eq!(records[0].job.name, "done-2", "最老的两条已结束记录被丢掉");
    let active = records.iter().filter(|r| r.state.is_active()).count();
    assert_eq!(active, 2, "进行中的绝不丢");
}

#[test]
fn percent_and_texts() {
    let mut record = row_from(1, 0, "", &job("a.mkv"));
    record.state = TransferState::Running;
    record.bytes = 512;
    record.total = Some(1024);
    assert_eq!(percent_of(&record), Some(50));
    let map = record_map(&record);
    assert_eq!(text_of(&map, "stateText"), "下载中");
    assert_eq!(text_of(&map, "progressText"), "50%");
    assert_eq!(text_of(&map, "sizeText"), "512 B / 1 KB");
    assert_eq!(text_of(&map, "path"), "", "未完成的行没有落盘路径");

    record.total = None;
    record.bytes = 2048;
    assert_eq!(percent_of(&record), None);
    let map = record_map(&record);
    assert_eq!(
        map.get(&QString::from("progress"))
            .and_then(|value| value.value::<i32>()),
        Some(-1),
        "总大小未知时进度用 -1，QML 显示不确定态"
    );
}

#[test]
fn state_names_are_stable_for_qml() {
    assert_eq!(TransferState::Queued.name(), "queued");
    assert_eq!(TransferState::Preparing.name(), "preparing");
    assert_eq!(TransferState::Running.name(), "running");
    assert_eq!(TransferState::Done.name(), "done");
    assert_eq!(TransferState::Failed.name(), "failed");
    assert_eq!(TransferState::Cancelled.name(), "cancelled");
}

#[test]
fn every_row_has_a_key_for_the_qml_diff() {
    // QML 的 ListModel 差异算法靠 key 识别行；任何一行缺 key 都会在插入时报
    // `<Unknown File>: key is undefined`，提示行还会被反复重复插入（2026-10-27）。
    let mut record = row_from(1, 0, "file-1", &job("a.mkv"));
    record.state = TransferState::Done;
    assert_eq!(
        text_of(&record_map(&record), "key"),
        "1:0",
        "普通记录的 key"
    );

    let notice = notice_map(5);
    assert_eq!(text_of(&notice, "key"), "notice", "提示行也必须有 key");
    assert!(
        text_of(&notice, "name").contains("未显示"),
        "提示行仍要带说明文字"
    );
    assert_eq!(
        text_of(&notice, "path"),
        "",
        "提示行也要给 required property path 一个空串"
    );
}

// —— 准备中 / 按 id 定位 / 文件夹展开（2026-10-27 下载并发修复）——

#[test]
fn preparing_counts_as_active() {
    let mut records = Vec::new();
    add_batch(&mut records, 1, &["big-folder"]);
    mark_preparing_in(&mut records, 1);
    assert_eq!(records[0].state, TransferState::Preparing);
    assert!(
        records[0].state.is_active(),
        "准备中要计入进行中（徽标 / 取消）"
    );
    assert!(!records[0].state.is_finished());

    let (_, active) = build_snapshot(&records);
    assert_eq!(active, 1, "进行中条数包含准备中");

    cancel_active_in(&mut records, None, "");
    assert_eq!(
        records[0].state,
        TransferState::Cancelled,
        "准备中的批次也要能被取消"
    );
}

#[test]
fn preparing_row_shows_explanation_and_unknown_size() {
    let mut records = Vec::new();
    add_batch(&mut records, 1, &["big-folder"]);
    mark_preparing_in(&mut records, 1);
    let map = record_map(&records[0]);
    assert_eq!(text_of(&map, "stateText"), "准备中");
    assert_eq!(text_of(&map, "detail"), "正在读取目录结构…");
    assert_eq!(text_of(&map, "sizeText"), "—", "还没读目录，大小未知");
}

#[test]
fn expand_folder_replaces_placeholder_in_place() {
    let mut records = Vec::new();
    add_batch(&mut records, 1, &["big-folder"]);
    add_batch(&mut records, 2, &["later"]);
    mark_preparing_in(&mut records, 1);
    let outer = job("第 1 集.mkv");
    let seeds = vec![
        TransferJob {
            id: "file-a".into(),
            sub_dir: vec!["big-folder".into()],
            size: 10,
            ..outer.clone()
        },
        TransferJob {
            id: "file-b".into(),
            name: "第 2 集.mkv".into(),
            sub_dir: vec!["big-folder".into()],
            size: 20,
            ..outer
        },
    ];
    expand_folder_in(&mut records, 1, &seeds);

    let ordered_names: Vec<String> = ordered(&records)
        .iter()
        .map(|record| record.display_name())
        .collect();
    assert_eq!(
        ordered_names,
        vec!["big-folder/第 1 集.mkv", "big-folder/第 2 集.mkv", "later"],
        "整批原地展开，后面排队的批次仍在下面"
    );
    assert_eq!(records[0].state, TransferState::Queued);
    assert_eq!(records[1].index, 1);
    assert_eq!(records[0].source_id, "source", "展开出来的行也带来源 id");
    assert_eq!(records[0].job.id, "file-a", "行内保留文件 id 供重试");
    assert_eq!(records[0].job.sub_dir, vec!["big-folder"], "行内保留子目录");
}

#[test]
fn cancel_by_batch_leaves_other_batches_alone() {
    let mut records = Vec::new();
    add_batch(&mut records, 1, &["a"]);
    add_batch(&mut records, 2, &["b"]);
    mark_preparing_in(&mut records, 1);
    cancel_active_in(&mut records, Some(1), "");
    assert_eq!(records[0].state, TransferState::Cancelled);
    assert_eq!(
        records[1].state,
        TransferState::Queued,
        "按批号取消不能动别的批"
    );
}

#[test]
fn fail_batch_marks_whole_batch_failed() {
    let mut records = Vec::new();
    add_batch(&mut records, 1, &["big-folder"]);
    add_batch(&mut records, 2, &["later"]);
    mark_preparing_in(&mut records, 1);
    fail_batch_in(&mut records, 1, "目录读取失败");
    assert_eq!(records[0].state, TransferState::Failed);
    assert_eq!(records[0].detail, "目录读取失败");
    assert_eq!(
        records[1].state,
        TransferState::Queued,
        "后面的批次照常排队"
    );
}

#[test]
fn record_map_exposes_source_id_for_locating() {
    let record = row_from(7, 0, "cloud-entry-42", &job("a.mkv"));
    let map = record_map(&record);
    assert_eq!(text_of(&map, "sourceId"), "cloud-entry-42");
}

#[test]
fn done_row_detail_comes_from_path() {
    let mut record = row_from(7, 0, "file-1", &job("a.mkv"));
    record.path = Some(PathBuf::from("/home/me/Downloads/a.mkv"));
    record.state = TransferState::Done;
    record.detail = "旧字符串".into();

    let map = record_map(&record);
    assert_eq!(text_of(&map, "detail"), "保存到 /home/me/Downloads/a.mkv");
    assert_eq!(
        text_of(&map, "path"),
        "/home/me/Downloads/a.mkv",
        "QML 的「打开位置」用原始路径"
    );
}

#[test]
fn display_name_keeps_folder_hierarchy() {
    let nested = TransferJob {
        sub_dir: vec!["剧集".into(), "第一季".into()],
        ..job("第 1 集.mkv")
    };
    assert_eq!(nested.display_name(), "剧集/第一季/第 1 集.mkv");

    let mut record = row_from(1, 0, "file-1", &nested);
    assert_eq!(record.display_name(), "剧集/第一季/第 1 集.mkv");
    assert_eq!(
        text_of(&record_map(&record), "name"),
        "剧集/第一季/第 1 集.mkv",
        "QML 的 name 角色仍是显示路径"
    );

    let single = TransferJob {
        sub_dir: Vec::new(),
        ..job("第 1 集.mkv")
    };
    assert_eq!(single.display_name(), "第 1 集.mkv");
    record.job = single;
    assert_eq!(record.display_name(), "第 1 集.mkv");
}
