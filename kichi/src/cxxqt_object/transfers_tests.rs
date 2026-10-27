//! `transfers.rs` 的单元测试：记录表的状态迁移、排序、上限与显示字段。
//!
//! 拆出来只为守住行数软上限（实现本身约 350 行）。

use cxx_qt_lib::QString;

use super::*;
use crate::cxxqt_object::transfers_view::{percent_of, record_map, QVariantMap};

fn add_batch(records: &mut Vec<Transfer>, seq: u64, names: &[&str]) {
    let jobs: Vec<(String, u64)> = names.iter().map(|name| ((*name).to_owned(), 0)).collect();
    push_batch(records, seq, "source", &jobs);
}

fn set(records: &mut [Transfer], batch: u64, index: usize, state: TransferState) {
    if let Some(record) = find(records, batch, index) {
        record.state = state;
    }
}

fn names(records: &[Transfer]) -> Vec<&str> {
    records.iter().map(|record| record.name.as_str()).collect()
}

#[test]
fn active_rows_come_first_in_batch_order() {
    let mut records = Vec::new();
    add_batch(&mut records, 1, &["a", "b", "c"]);
    set(&mut records, 1, 0, TransferState::Done);
    set(&mut records, 1, 2, TransferState::Running);
    let ordered = ordered(&records);
    let names: Vec<&str> = ordered.iter().map(|record| record.name.as_str()).collect();
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
    let names: Vec<&str> = ordered.iter().map(|record| record.name.as_str()).collect();
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
fn finishing_records_detail_and_size() {
    let mut records = Vec::new();
    add_batch(&mut records, 1, &["a", "b"]);
    begin_in(&mut records, 1, 0);
    progress_in(&mut records, 1, 0, 1024, Some(1024));
    finish_ok_in(&mut records, 1, 0, "/home/me/Downloads/a.mkv");
    assert_eq!(records[0].state, TransferState::Done);
    assert_eq!(records[0].detail, "保存到 /home/me/Downloads/a.mkv");
    assert_eq!(records[0].bytes, 1024);

    begin_in(&mut records, 1, 1);
    finish_err_in(&mut records, 1, 1, "下载中断：连接被重置");
    assert_eq!(records[1].state, TransferState::Failed);
    assert_eq!(records[1].detail, "下载中断：连接被重置");
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
        records.push(Transfer {
            batch: 1,
            index,
            name: format!("done-{index}"),
            source_id: String::new(),
            state: TransferState::Done,
            bytes: 0,
            total: None,
            detail: String::new(),
        });
    }
    for index in 0..2 {
        records.push(Transfer {
            batch: 1,
            index: MAX_RECORDS + index,
            name: format!("active-{index}"),
            source_id: String::new(),
            state: TransferState::Running,
            bytes: 0,
            total: None,
            detail: String::new(),
        });
    }
    trim(&mut records);

    assert_eq!(records.len(), MAX_RECORDS);
    assert_eq!(records[0].name, "done-2", "最老的两条已结束记录被丢掉");
    let active = records.iter().filter(|r| r.state.is_active()).count();
    assert_eq!(active, 2, "进行中的绝不丢");
}

#[test]
fn percent_and_texts() {
    let mut record = Transfer {
        batch: 1,
        index: 0,
        name: "a.mkv".into(),
        source_id: String::new(),
        state: TransferState::Running,
        bytes: 512,
        total: Some(1024),
        detail: String::new(),
    };
    assert_eq!(percent_of(&record), Some(50));
    let map = record_map(&record);
    let get = |key: &str| {
        map.get(&QString::from(key))
            .and_then(|value| value.value::<QString>())
            .map(|value| value.to_string())
            .unwrap_or_default()
    };
    assert_eq!(get("stateText"), "下载中");
    assert_eq!(get("progressText"), "50%");
    assert_eq!(get("sizeText"), "512 B / 1 KB");

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
    fn key_of(map: &QVariantMap) -> String {
        map.get(&QString::from("key"))
            .and_then(|value| value.value::<QString>())
            .map(|value| value.to_string())
            .unwrap_or_default()
    }

    let record = Transfer {
        batch: 1,
        index: 0,
        name: "a.mkv".into(),
        source_id: "file-1".into(),
        state: TransferState::Done,
        bytes: 0,
        total: None,
        detail: String::new(),
    };
    assert_eq!(key_of(&record_map(&record)), "1:0", "普通记录的 key");

    let notice = notice_map(5);
    assert_eq!(key_of(&notice), "notice", "提示行也必须有 key");
    assert!(
        notice
            .get(&QString::from("name"))
            .and_then(|value| value.value::<QString>())
            .is_some_and(|name| name.to_string().contains("未显示")),
        "提示行仍要带说明文字"
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
    let get = |key: &str| {
        map.get(&QString::from(key))
            .and_then(|value| value.value::<QString>())
            .map(|value| value.to_string())
            .unwrap_or_default()
    };
    assert_eq!(get("stateText"), "准备中");
    assert_eq!(get("detail"), "正在读取目录结构…");
    assert_eq!(get("sizeText"), "—", "还没读目录，大小未知");
}

#[test]
fn expand_folder_replaces_placeholder_in_place() {
    let mut records = Vec::new();
    add_batch(&mut records, 1, &["big-folder"]);
    add_batch(&mut records, 2, &["later"]);
    mark_preparing_in(&mut records, 1);
    let jobs = [
        ("big-folder/a.mkv".to_owned(), 10),
        ("big-folder/b.mkv".to_owned(), 20),
    ];
    expand_folder_in(&mut records, 1, &jobs);

    let ordered_names: Vec<&str> = ordered(&records).iter().map(|r| r.name.as_str()).collect();
    assert_eq!(
        ordered_names,
        vec!["big-folder/a.mkv", "big-folder/b.mkv", "later"],
        "整批原地展开，后面排队的批次仍在下面"
    );
    assert_eq!(records[0].state, TransferState::Queued);
    assert_eq!(records[1].index, 1);
    assert_eq!(records[0].source_id, "source", "展开出来的行也带来源 id");
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
    let record = Transfer {
        batch: 7,
        index: 0,
        name: "剧集/a.mkv".into(),
        source_id: "cloud-entry-42".into(),
        state: TransferState::Running,
        bytes: 0,
        total: None,
        detail: String::new(),
    };
    let map = record_map(&record);
    let source = map
        .get(&QString::from("sourceId"))
        .and_then(|value| value.value::<QString>())
        .map(|value| value.to_string());
    assert_eq!(source.as_deref(), Some("cloud-entry-42"));
}
