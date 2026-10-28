//! `transfers.rs` 的单元测试：记录表的状态迁移、父行聚合、排序、上限与显示字段。
//!
//! 拆出来只为守住行数软上限（实现本身约 350 行）。文件夹批次在主列表
//! 只占一条总账行，子行的字段与详情快照也在这里覆盖。

use std::path::{Path, PathBuf};
use std::time::Instant;

use cxx_qt_lib::QString;
use kichi_core::EntryKind;

use super::*;
use crate::cxxqt_object::transfers_view::{
    batch_stats, build_detail_snapshot, build_history_snapshot, build_snapshot, day_key,
    history_days, live_map, notice_map, ordered, parent_live_map, percent_of, record_map,
    QVariantMap, HISTORY_DAY_ALL, MAX_ROWS,
};
use crate::cxxqt_object::QVariantList;

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

fn folder_job(name: &str) -> TransferJob {
    TransferJob {
        kind: EntryKind::Folder,
        ..job(name)
    }
}

fn add_batch(records: &mut Vec<Transfer>, seq: u64, names: &[&str]) {
    for (index, name) in names.iter().enumerate() {
        records.push(row_from(seq, index, "source", &job(name), 1_790_000_000, 1));
    }
}

/// 一个文件夹批次：父行 + 读目录后挂上的子行。
fn folder_batch(records: &mut Vec<Transfer>, batch: u64, name: &str, children: &[(&str, u64)]) {
    records.push(row_from(
        batch,
        0,
        "source",
        &folder_job(name),
        1_790_000_000,
        1,
    ));
    mark_preparing_in(records, batch);
    let base = job("seed");
    let seeds: Vec<TransferJob> = children
        .iter()
        .map(|(child, size)| TransferJob {
            id: format!("{name}-{child}"),
            name: (*child).to_owned(),
            sub_dir: vec![name.to_owned()],
            size: *size,
            ..base.clone()
        })
        .collect();
    expand_folder_in(records, batch, &seeds, 1_790_000_000, 1);
}

fn child_at(records: &[Transfer], batch: u64, index: usize) -> usize {
    records
        .iter()
        .position(|record| record.batch == batch && record.child && record.index == index)
        .expect("子行存在")
}

fn child_mut(records: &mut [Transfer], batch: u64, index: usize) -> &mut Transfer {
    let at = records
        .iter()
        .position(|record| record.batch == batch && record.child && record.index == index)
        .expect("子行存在");
    &mut records[at]
}

fn set(records: &mut [Transfer], batch: u64, index: usize, state: TransferState) {
    if let Some(record) = find_file(records, batch, index) {
        record.state = state;
    }
}

fn set_child(records: &mut [Transfer], batch: u64, index: usize, state: TransferState) {
    let at = child_at(records, batch, index);
    records[at].state = state;
}

fn text_of(map: &QVariantMap, key: &str) -> String {
    map.get(&QString::from(key))
        .and_then(|value| value.value::<QString>())
        .map(|value| value.to_string())
        .unwrap_or_default()
}

fn int_of(map: &QVariantMap, key: &str) -> i32 {
    map.get(&QString::from(key))
        .and_then(|value| value.value::<i32>())
        .unwrap_or_default()
}

fn bool_of(map: &QVariantMap, key: &str) -> bool {
    map.get(&QString::from(key))
        .and_then(|value| value.value::<bool>())
        .unwrap_or_default()
}

fn map_at(list: &QVariantList, index: usize) -> QVariantMap {
    list.at(index)
        .and_then(|value| value.value::<QVariantMap>())
        .expect("快照的每一行都是 map")
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
fn clear_finished_only_drops_finished_batches() {
    let mut records = Vec::new();
    add_batch(&mut records, 1, &["done-a"]);
    set(&mut records, 1, 0, TransferState::Done);
    add_batch(&mut records, 2, &["active"]);
    clear_finished_in(&mut records);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].state, TransferState::Queued);
    assert_eq!(records[0].batch, 2, "只清整批都结束的组");
}

#[test]
fn clear_finished_keeps_active_folder_with_done_children() {
    let mut records = Vec::new();
    folder_batch(&mut records, 1, "剧集", &[("a.mkv", 10), ("b.mkv", 10)]);
    set_child(&mut records, 1, 0, TransferState::Done);
    begin_in(&mut records, 1, 1);
    clear_finished_in(&mut records);
    assert_eq!(records.len(), 3, "正在下载的文件夹与已完成的子行都保留");
    assert_eq!(records[0].state, TransferState::Running);
}

#[test]
fn trim_evicts_whole_oldest_finished_batches() {
    let mut records = Vec::new();
    for batch in 1..=MAX_RECORDS as u64 {
        let mut record = row_from(
            batch,
            0,
            "",
            &job(&format!("done-{batch}")),
            1_790_000_000,
            1,
        );
        record.state = TransferState::Done;
        records.push(record);
    }
    for batch in MAX_RECORDS as u64 + 1..=MAX_RECORDS as u64 + 2 {
        let mut record = row_from(
            batch,
            0,
            "",
            &job(&format!("active-{batch}")),
            1_790_000_000,
            1,
        );
        record.state = TransferState::Running;
        records.push(record);
    }
    trim(&mut records);

    assert_eq!(records.len(), MAX_RECORDS);
    assert_eq!(records[0].batch, 3, "最老的两批已结束记录被整批丢掉");
    let active = records.iter().filter(|r| r.state.is_active()).count();
    assert_eq!(active, 2, "进行中的绝不丢");
}

#[test]
fn trim_keeps_newest_finished_batch_even_over_limit() {
    let mut records = Vec::new();
    records.push(row_from(
        1,
        0,
        "",
        &folder_job("超大文件夹"),
        1_790_000_000,
        1,
    ));
    records[0].state = TransferState::Done;
    for index in 0..MAX_RECORDS + 100 {
        let mut child = row_from(1, index, "", &job(&format!("f{index}")), 1_790_000_000, 1);
        child.child = true;
        child.state = TransferState::Done;
        records.push(child);
    }
    let total = records.len();
    trim(&mut records);
    assert_eq!(records.len(), total, "只剩最新一个已结束批次时保留它");
}

#[test]
fn percent_and_texts() {
    let mut record = row_from(1, 0, "", &job("a.mkv"), 1_790_000_000, 1);
    record.state = TransferState::Running;
    record.bytes = 512;
    record.total = Some(1024);
    assert_eq!(percent_of(&record), Some(50));
    let map = record_map(&record, &[]);
    assert_eq!(text_of(&map, "stateText"), "下载中");
    assert_eq!(text_of(&map, "progressText"), "50%");
    assert_eq!(text_of(&map, "sizeText"), "512 B / 1 KB");
    assert_eq!(text_of(&map, "path"), "", "未完成的行没有落盘路径");
    assert!(!bool_of(&map, "folder"));
    assert_eq!(int_of(&map, "childCount"), 0);

    record.total = None;
    record.bytes = 2048;
    assert_eq!(percent_of(&record), None);
    let map = record_map(&record, &[]);
    assert_eq!(int_of(&map, "progress"), -1, "总大小未知时进度用 -1");
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
    let mut record = row_from(1, 0, "file-1", &job("a.mkv"), 1_790_000_000, 1);
    record.state = TransferState::Done;
    assert_eq!(text_of(&record_map(&record, &[]), "key"), "1:0");

    let mut records = Vec::new();
    folder_batch(&mut records, 1, "剧集", &[("a.mkv", 10)]);
    let parent = record_map(&records[0], &records);
    assert_eq!(text_of(&parent, "key"), "1:folder", "父行固定 batch:folder");
    assert_eq!(
        text_of(&map_at(&build_detail_snapshot(&records, 1), 0), "key"),
        "1:0",
        "子行仍然是 batch:index"
    );

    let notice = notice_map(5);
    assert_eq!(text_of(&notice, "key"), "notice", "提示行也必须有 key");
    assert!(
        text_of(&notice, "name").contains("未显示"),
        "提示行仍要带说明文字"
    );
    assert_eq!(text_of(&notice, "path"), "", "提示行也要给 path 空串");
    assert!(!bool_of(&notice, "folder"));
    assert_eq!(int_of(&notice, "childCount"), 0);
}

// —— 准备中 / 文件夹展开 / 父行聚合（2026-10-27 文件夹详情页）——

#[test]
fn preparing_counts_as_active() {
    let mut records = Vec::new();
    records.push(row_from(
        1,
        0,
        "src",
        &folder_job("big-folder"),
        1_790_000_000,
        1,
    ));
    mark_preparing_in(&mut records, 1);
    assert_eq!(records[0].state, TransferState::Preparing);
    assert!(records[0].state.is_active(), "准备中要计入进行中");
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
    records.push(row_from(
        1,
        0,
        "src",
        &folder_job("big-folder"),
        1_790_000_000,
        1,
    ));
    mark_preparing_in(&mut records, 1);
    let map = record_map(&records[0], &records);
    assert_eq!(text_of(&map, "stateText"), "准备中");
    assert_eq!(text_of(&map, "detail"), "正在读取目录结构…");
    assert_eq!(text_of(&map, "sizeText"), "—", "还没读目录，大小未知");
    assert!(bool_of(&map, "folder"));
    assert_eq!(int_of(&map, "childCount"), 0);
}

#[test]
fn folder_expansion_keeps_ledger_row_and_children() {
    let mut records = Vec::new();
    folder_batch(
        &mut records,
        1,
        "剧集",
        &[("第 1 集.mkv", 10), ("第 2 集.mkv", 20)],
    );
    add_batch(&mut records, 2, &["later"]);

    assert_eq!(records.len(), 4, "父行 + 2 个子行 + 后面排队的批次");
    assert!(is_folder_ledger(&records[0]), "父行留在原位");
    assert_eq!(records[0].state, TransferState::Preparing);
    assert_eq!(records[1].index, 0);
    assert!(records[1].child && records[2].child);
    assert_eq!(records[3].job.name, "later", "后面排队的批次不受影响");
    assert_eq!(records[1].source_id, "source", "子行也带来源 id");
    assert_eq!(records[1].job.sub_dir, vec!["剧集"], "子行保留子目录");

    let (list, active) = build_snapshot(&records);
    assert_eq!(active, 2, "文件夹批次只算一个进行中");
    assert_eq!(list.len(), 2, "主列表只有父行 + 单文件批次");
    assert_eq!(text_of(&map_at(&list, 0), "key"), "1:folder");
    assert_eq!(text_of(&map_at(&list, 1), "key"), "2:0");

    let detail = build_detail_snapshot(&records, 1);
    assert_eq!(detail.len(), 2);
    assert_eq!(text_of(&map_at(&detail, 0), "name"), "第 1 集.mkv");
    assert_eq!(text_of(&map_at(&detail, 1), "name"), "第 2 集.mkv");
}

#[test]
fn find_file_never_confuses_parent_and_first_child() {
    let mut records = Vec::new();
    folder_batch(&mut records, 7, "剧集", &[("第 1 集.mkv", 10)]);
    assert_eq!(record_key(&records[0]), "7:folder");
    assert_eq!(record_key(&records[1]), "7:0");

    let found = find_file(&mut records, 7, 0).expect("能找到第一个子行");
    assert!(found.child, "按 (batch, 0) 查文件必须拿到子行，而不是父行");

    // 进度只落到子行上；父行自己的 bytes / total 不被写
    begin_in(&mut records, 7, 0);
    progress_in(&mut records, 7, 0, 5, Some(10));
    assert_eq!(records[1].bytes, 5);
    assert_eq!(records[0].bytes, 0);
}

#[test]
fn parent_state_follows_children() {
    let mut records = Vec::new();
    folder_batch(&mut records, 1, "剧集", &[("a.mkv", 10), ("b.mkv", 10)]);
    assert_eq!(
        records[0].state,
        TransferState::Preparing,
        "子行入表时仍准备中"
    );

    begin_in(&mut records, 1, 0);
    assert_eq!(records[0].state, TransferState::Running, "有子行开始下载");

    finish_ok_in(&mut records, 1, 0, Path::new("/tmp/a.mkv"));
    assert_eq!(records[0].state, TransferState::Running, "还有子行排队");

    finish_ok_in(&mut records, 1, 1, Path::new("/tmp/b.mkv"));
    assert_eq!(records[0].state, TransferState::Done, "全部完成");

    let mut failed = Vec::new();
    folder_batch(&mut failed, 2, "坏文件夹", &[("a.mkv", 10), ("b.mkv", 10)]);
    begin_in(&mut failed, 2, 0);
    finish_err_in(&mut failed, 2, 0, "坏链");
    finish_err_in(&mut failed, 2, 1, "坏链");
    assert_eq!(failed[0].state, TransferState::Failed, "全部失败");
    assert_eq!(
        text_of(&record_map(&failed[0], &failed), "stateText"),
        "失败",
        "一个都没成功时不叫部分失败"
    );
}

#[test]
fn cancel_is_not_revived_by_late_completion() {
    let mut records = Vec::new();
    folder_batch(&mut records, 1, "剧集", &[("a.mkv", 10), ("b.mkv", 10)]);
    begin_in(&mut records, 1, 0);
    cancel_active_in(&mut records, None, "");
    assert_eq!(records[0].state, TransferState::Cancelled);

    // 取消瞬间当前文件刚好下载完整：子行记完成，父行不能被改回 Running / Done
    finish_ok_in(&mut records, 1, 0, Path::new("/tmp/a.mkv"));
    assert_eq!(records[0].state, TransferState::Cancelled);
    assert_eq!(records[1].state, TransferState::Done);
}

#[test]
fn folder_row_aggregates_progress_and_counts() {
    let mut records = Vec::new();
    folder_batch(
        &mut records,
        1,
        "剧集",
        &[("a.mkv", 10), ("b.mkv", 20), ("c.mkv", 30)],
    );
    set_child(&mut records, 1, 0, TransferState::Done);
    child_mut(&mut records, 1, 0).bytes = 10;
    begin_in(&mut records, 1, 1);
    progress_in(&mut records, 1, 1, 5, Some(20));

    let stats = batch_stats(&records, 1);
    assert_eq!(stats.total, 3);
    assert_eq!(stats.done, 1);
    assert_eq!(stats.progress_bytes, 15);
    assert_eq!(stats.total_bytes, Some(60));
    assert_eq!(stats.running_position, Some(2));
    assert_eq!(stats.running_name, "b.mkv");

    let map = record_map(&records[0], &records);
    assert_eq!(text_of(&map, "stateText"), "下载中");
    assert_eq!(int_of(&map, "progress"), 25);
    assert_eq!(text_of(&map, "progressText"), "25% · 2/3");
    assert_eq!(text_of(&map, "sizeText"), "15 B / 60 B");
    assert_eq!(int_of(&map, "childCount"), 3);
    assert_eq!(int_of(&map, "failedCount"), 0);
    assert!(bool_of(&map, "folder"));

    finish_err_in(&mut records, 1, 1, "坏链");
    finish_err_in(&mut records, 1, 2, "坏链");
    assert_eq!(records[0].state, TransferState::Failed);
    let map = record_map(&records[0], &records);
    assert_eq!(text_of(&map, "stateText"), "部分失败");
    assert_eq!(text_of(&map, "detail"), "成功 1 个 · 失败 2 个");
    assert_eq!(text_of(&map, "sizeText"), "60 B");
    assert_eq!(int_of(&map, "failedCount"), 2);
}

#[test]
fn folder_row_keeps_unknown_total_indeterminate() {
    let mut records = Vec::new();
    folder_batch(&mut records, 1, "剧集", &[("a.mkv", 0), ("b.mkv", 20)]);
    begin_in(&mut records, 1, 1);
    progress_in(&mut records, 1, 1, 5, Some(20));

    let stats = batch_stats(&records, 1);
    assert_eq!(stats.total_bytes, None, "有一个子行大小未知");
    let map = record_map(&records[0], &records);
    assert_eq!(int_of(&map, "progress"), -1);
    assert_eq!(
        text_of(&map, "progressText"),
        "2/2",
        "总量未知时至少给当前位置"
    );
    assert_eq!(text_of(&map, "sizeText"), "5 B");
}

#[test]
fn zero_byte_children_do_not_make_total_unknown() {
    let mut records = Vec::new();
    folder_batch(&mut records, 1, "剧集", &[("empty.txt", 0), ("a.mkv", 10)]);
    set_child(&mut records, 1, 0, TransferState::Done);
    child_mut(&mut records, 1, 0).total = Some(0);
    set_child(&mut records, 1, 1, TransferState::Done);
    records[0].state = TransferState::Done;

    let stats = batch_stats(&records, 1);
    assert_eq!(stats.total_bytes, Some(10), "0 字节按 0 处理，不算未知");
    assert_eq!(stats.done, 2);
    assert_eq!(int_of(&record_map(&records[0], &records), "progress"), 100);
}

#[test]
fn folder_cancelled_row_text() {
    let mut records = Vec::new();
    folder_batch(&mut records, 1, "剧集", &[("a.mkv", 10), ("b.mkv", 10)]);
    set_child(&mut records, 1, 0, TransferState::Done);
    set_child(&mut records, 1, 1, TransferState::Cancelled);
    records[0].state = TransferState::Cancelled;

    let map = record_map(&records[0], &records);
    assert_eq!(text_of(&map, "stateText"), "已取消");
    assert_eq!(text_of(&map, "detail"), "已完成 1 个 · 已取消");
}

#[test]
fn main_snapshot_counts_a_folder_as_one_row() {
    let mut records = Vec::new();
    folder_batch(&mut records, 1, "剧集", &[("a.mkv", 10), ("b.mkv", 10)]);
    add_batch(&mut records, 2, &["single.mkv"]);
    begin_in(&mut records, 2, 0);
    // 父行也随子行进入进行中
    begin_in(&mut records, 1, 0);

    let (list, active) = build_snapshot(&records);
    assert_eq!(list.len(), 2, "子行不出现在主列表");
    assert_eq!(active, 2, "进行中按批次算：文件夹算 1");
    assert_eq!(text_of(&map_at(&list, 0), "key"), "1:folder");
    assert_eq!(text_of(&map_at(&list, 1), "key"), "2:0");
}

#[test]
fn main_snapshot_caps_at_500_batches_and_adds_notice() {
    let mut records = Vec::new();
    for batch in 1..=MAX_ROWS as u64 + 1 {
        let mut record = row_from(batch, 0, "", &job(&format!("f{batch}")), 1_790_000_000, 1);
        record.state = TransferState::Done;
        records.push(record);
    }
    let (list, active) = build_snapshot(&records);
    assert_eq!(list.len(), MAX_ROWS + 1, "500 行 + 1 条提示");
    assert_eq!(active, 0);
    assert_eq!(text_of(&map_at(&list, MAX_ROWS), "key"), "notice");
    assert!(
        text_of(&map_at(&list, MAX_ROWS), "name").contains("1 条记录未显示"),
        "提示剩下的批次数量"
    );
}

#[test]
fn detail_snapshot_caps_at_500_files_and_adds_notice() {
    let mut records = Vec::new();
    records.push(row_from(
        1,
        0,
        "",
        &folder_job("超大文件夹"),
        1_790_000_000,
        1,
    ));
    for index in 0..MAX_ROWS + 1 {
        let mut child = row_from(1, index, "", &job(&format!("f{index}")), 1_790_000_000, 1);
        child.child = true;
        records.push(child);
    }
    let detail = build_detail_snapshot(&records, 1);
    assert_eq!(detail.len(), MAX_ROWS + 1, "500 个文件 + 1 条提示");
    assert_eq!(text_of(&map_at(&detail, MAX_ROWS), "key"), "notice");
    assert!(
        text_of(&map_at(&detail, MAX_ROWS), "name").contains("1 个文件未显示"),
        "提示剩下的文件数量"
    );
}

#[test]
fn detail_snapshot_keeps_download_order() {
    let mut records = Vec::new();
    folder_batch(
        &mut records,
        5,
        "剧集",
        &[
            ("第 1 集.mkv", 10),
            ("第 2 集.mkv", 10),
            ("第 3 集.mkv", 10),
        ],
    );
    set_child(&mut records, 5, 0, TransferState::Done);
    set_child(&mut records, 5, 1, TransferState::Failed);
    begin_in(&mut records, 5, 2);

    let keys = |records: &[Transfer]| -> Vec<String> {
        let detail = build_detail_snapshot(records, 5);
        (0..detail.len())
            .map(|index| text_of(&map_at(&detail, index), "key"))
            .collect()
    };
    assert_eq!(
        keys(&records)
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["5:0", "5:1", "5:2"],
        "固定下载顺序"
    );

    // 第一个子行完成后仍然留在第一位（详情页不按状态重排）
    set_child(&mut records, 5, 0, TransferState::Done);
    assert_eq!(
        keys(&records)
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["5:0", "5:1", "5:2"]
    );
}

#[test]
fn detail_snapshot_names_are_relative_to_folder() {
    let mut records = Vec::new();
    records.push(row_from(5, 0, "src", &folder_job("剧集"), 1_790_000_000, 1));
    let base = job("第 01 集.mkv");
    let seeds = vec![
        TransferJob {
            id: "nested".into(),
            sub_dir: vec!["剧集".into(), "第一季".into()],
            size: 10,
            ..base.clone()
        },
        TransferJob {
            id: "flat".into(),
            sub_dir: vec!["剧集".into()],
            size: 10,
            ..base
        },
    ];
    expand_folder_in(&mut records, 5, &seeds, 1_790_000_000, 1);

    let detail = build_detail_snapshot(&records, 5);
    assert_eq!(text_of(&map_at(&detail, 0), "name"), "第一季/第 01 集.mkv");
    assert_eq!(text_of(&map_at(&detail, 1), "name"), "第 01 集.mkv");
}

#[test]
fn parent_path_requires_a_done_child() {
    let mut records = Vec::new();
    folder_batch(&mut records, 1, "剧集", &[("a.mkv", 10)]);
    assert_eq!(
        text_of(&record_map(&records[0], &records), "path"),
        "",
        "还没有子文件落盘时父行不可定位"
    );

    set_child(&mut records, 1, 0, TransferState::Done);
    child_mut(&mut records, 1, 0).path = Some(PathBuf::from("/tmp/剧集/a.mkv"));
    records[0].state = TransferState::Done;
    let expected = PathBuf::from("/tmp").join("剧集");
    assert_eq!(
        text_of(&record_map(&records[0], &records), "path"),
        expected.display().to_string(),
        "至少一个子文件完成后给文件夹路径"
    );
    assert_eq!(
        text_of(&record_map(&records[0], &records), "detail"),
        format!("1 个文件 · 保存到 {}", expected.display())
    );
}

#[test]
fn live_maps_carry_file_and_parent_progress() {
    let mut records = Vec::new();
    folder_batch(&mut records, 1, "剧集", &[("a.mkv", 10), ("b.mkv", 20)]);
    set_child(&mut records, 1, 0, TransferState::Done);
    child_mut(&mut records, 1, 0).bytes = 10;
    begin_in(&mut records, 1, 1);
    progress_in(&mut records, 1, 1, 5, Some(20));

    let child = live_map(&records[child_at(&records, 1, 1)]);
    assert_eq!(text_of(&child, "key"), "1:1");
    assert_eq!(int_of(&child, "progress"), 25);

    let parent = parent_live_map(&records[0], &records);
    assert_eq!(text_of(&parent, "key"), "1:folder");
    assert_eq!(int_of(&parent, "progress"), 50, "15 / 30");
    assert_eq!(text_of(&parent, "progressText"), "50% · 2/2");
    assert_eq!(text_of(&parent, "sizeText"), "15 B / 30 B");
}

#[test]
fn record_map_exposes_source_id_for_locating() {
    let record = row_from(7, 0, "cloud-entry-42", &job("a.mkv"), 1_790_000_000, 1);
    let map = record_map(&record, &[]);
    assert_eq!(text_of(&map, "sourceId"), "cloud-entry-42");
}

#[test]
fn done_row_detail_comes_from_path() {
    let mut record = row_from(7, 0, "file-1", &job("a.mkv"), 1_790_000_000, 1);
    record.path = Some(PathBuf::from("/home/me/Downloads/a.mkv"));
    record.state = TransferState::Done;
    record.detail = "旧字符串".into();

    let map = record_map(&record, &[]);
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

    let mut record = row_from(1, 0, "file-1", &nested, 1_790_000_000, 1);
    assert_eq!(record.display_name(), "剧集/第一季/第 1 集.mkv");
    assert_eq!(
        text_of(&record_map(&record, &[]), "name"),
        "剧集/第一季/第 1 集.mkv",
        "主列表的单文件行仍用完整显示路径"
    );

    let single = TransferJob {
        sub_dir: Vec::new(),
        ..job("第 1 集.mkv")
    };
    assert_eq!(single.display_name(), "第 1 集.mkv");
    record.job = single;
    assert_eq!(record.display_name(), "第 1 集.mkv");
}

// —— 空文件夹与跳过目录（2026-10-27 文件夹下载边界）——

/// 空文件夹批次：父行没有子行，建完目录直接标完成。
fn empty_folder_batch(records: &mut Vec<Transfer>, batch: u64, name: &str) {
    records.push(row_from(
        batch,
        0,
        "source",
        &folder_job(name),
        1_790_000_000,
        1,
    ));
    mark_preparing_in(records, batch);
    finish_folder_empty_in(records, batch);
}

#[test]
fn empty_folder_row_is_done_without_children() {
    let mut records = Vec::new();
    empty_folder_batch(&mut records, 1, "空文件夹");
    assert_eq!(records.len(), 1, "空文件夹只有总账行，没有子行");
    assert_eq!(records[0].state, TransferState::Done);
    assert!(records[0].empty_folder);

    let map = record_map(&records[0], &records);
    let expected = PathBuf::from("/tmp").join("空文件夹");
    assert_eq!(text_of(&map, "stateText"), "已完成");
    assert_eq!(int_of(&map, "progress"), 100, "完成的父行进度 100");
    assert_eq!(text_of(&map, "sizeText"), "—");
    assert_eq!(
        text_of(&map, "detail"),
        format!("空文件夹 · 保存到 {}", expected.display())
    );
    assert_eq!(
        text_of(&map, "path"),
        expected.display().to_string(),
        "空文件夹也要能「打开位置」"
    );
    assert_eq!(int_of(&map, "childCount"), 0);
    assert!(bool_of(&map, "folder"));
}

#[test]
fn empty_folder_row_mentions_skipped_directories() {
    let mut records = Vec::new();
    empty_folder_batch(&mut records, 1, "剧集");
    set_skipped_in(&mut records, 1, 2);

    let map = record_map(&records[0], &records);
    assert!(
        text_of(&map, "detail").ends_with("· 跳过 2 个目录"),
        "空文件夹也带跳过后缀：{}",
        text_of(&map, "detail")
    );
}

#[test]
fn empty_folder_finish_does_not_resurrect_a_cancelled_row() {
    let mut records = Vec::new();
    records.push(row_from(1, 0, "src", &folder_job("剧集"), 1_790_000_000, 1));
    mark_preparing_in(&mut records, 1);
    cancel_active_in(&mut records, Some(1), "");
    finish_folder_empty_in(&mut records, 1);
    assert_eq!(
        records[0].state,
        TransferState::Cancelled,
        "取消的批次不改回完成"
    );
}

#[test]
fn done_row_mentions_skipped_directories() {
    let mut records = Vec::new();
    folder_batch(&mut records, 1, "剧集", &[("a.mkv", 10), ("b.mkv", 10)]);
    set_child(&mut records, 1, 0, TransferState::Done);
    set_child(&mut records, 1, 1, TransferState::Done);
    records[0].state = TransferState::Done;
    set_skipped_in(&mut records, 1, 2);

    let map = record_map(&records[0], &records);
    let detail = text_of(&map, "detail");
    assert!(detail.starts_with("2 个文件 · 保存到"), "{detail}");
    assert!(detail.ends_with("· 跳过 2 个目录"), "{detail}");
}

#[test]
fn skipped_suffix_appears_on_failed_cancelled_and_running_rows() {
    let mut failed = Vec::new();
    folder_batch(&mut failed, 1, "剧集", &[("a.mkv", 10), ("b.mkv", 10)]);
    set_child(&mut failed, 1, 0, TransferState::Done);
    finish_err_in(&mut failed, 1, 1, "断网");
    set_skipped_in(&mut failed, 1, 1);
    let detail = text_of(&record_map(&failed[0], &failed), "detail");
    assert_eq!(detail, "成功 1 个 · 失败 1 个 · 跳过 1 个目录");

    let mut cancelled = Vec::new();
    folder_batch(&mut cancelled, 2, "剧集", &[("a.mkv", 10)]);
    set_child(&mut cancelled, 2, 0, TransferState::Done);
    cancel_active_in(&mut cancelled, Some(2), "");
    set_skipped_in(&mut cancelled, 2, 3);
    let detail = text_of(&record_map(&cancelled[0], &cancelled), "detail");
    assert_eq!(detail, "已完成 1 个 · 已取消 · 跳过 3 个目录");

    let mut running = Vec::new();
    folder_batch(&mut running, 3, "剧集", &[("a.mkv", 10), ("b.mkv", 10)]);
    begin_in(&mut running, 3, 0);
    set_skipped_in(&mut running, 3, 1);
    let detail = text_of(&record_map(&running[0], &running), "detail");
    assert_eq!(detail, "第 1/2 个 · 跳过 1 个目录");
}

#[test]
fn no_skip_count_means_no_suffix() {
    let mut records = Vec::new();
    folder_batch(&mut records, 1, "剧集", &[("a.mkv", 10)]);
    set_child(&mut records, 1, 0, TransferState::Done);
    records[0].state = TransferState::Done;
    let map = record_map(&records[0], &records);
    assert!(!text_of(&map, "detail").contains("跳过"));
    assert!(
        !text_of(&record_map(&records[0], &records), "path").is_empty(),
        "有子文件完成时仍有路径"
    );
}

#[test]
fn skipped_counts_only_live_on_the_folder_ledger() {
    let mut records = Vec::new();
    folder_batch(&mut records, 1, "剧集", &[("a.mkv", 10)]);
    set_skipped_in(&mut records, 1, 4);
    add_skipped_in(&mut records, 1, 2);
    assert_eq!(records[0].skipped, 6, "总账行累计跳过数");
    assert_eq!(records[1].skipped, 0, "子行不记跳过数");
}

#[test]
fn failed_empty_batch_has_no_path() {
    let mut records = Vec::new();
    records.push(row_from(1, 0, "src", &folder_job("剧集"), 1_790_000_000, 1));
    mark_preparing_in(&mut records, 1);
    fail_batch_in(&mut records, 1, "所有子目录都无法读取（跳过 2 个）");

    let map = record_map(&records[0], &records);
    assert_eq!(text_of(&map, "path"), "", "没建目录就没有可打开的位置");
    assert_eq!(text_of(&map, "detail"), "所有子目录都无法读取（跳过 2 个）");
    assert_eq!(int_of(&map, "progress"), 0);
}

// —— 已中断 / 日期分组 / 清空全部（2026-10-28 下载历史落盘）——

#[test]
fn interrupted_state_text_and_name() {
    assert_eq!(TransferState::Interrupted.name(), "interrupted");
    assert_eq!(TransferState::Interrupted.text(), "已中断");
    assert!(
        !TransferState::Interrupted.is_active(),
        "已中断不占调度槽、也不能取消"
    );
    assert!(
        !TransferState::Interrupted.is_finished(),
        "已中断不算结束：清空记录不清它"
    );
}

#[test]
fn interrupted_parent_survives_child_state_changes() {
    let mut records = Vec::new();
    folder_batch(&mut records, 1, "剧集", &[("a.mkv", 10), ("b.mkv", 10)]);
    records[0].state = TransferState::Interrupted;
    set_child(&mut records, 1, 0, TransferState::Interrupted);
    set_child(&mut records, 1, 1, TransferState::Interrupted);

    // 父行是已中断时，子行状态变化不能把它改回失败 / 完成
    finish_err_in(&mut records, 1, 0, "旧原因");
    assert_eq!(records[0].state, TransferState::Interrupted);

    let map = record_map(&records[0], &records);
    assert_eq!(text_of(&map, "stateText"), "已中断");
    assert_eq!(text_of(&map, "path"), "", "已中断的文件夹没有可打开的位置");
    assert!(
        text_of(&map, "detail").contains("上次退出时未完成"),
        "说明要写清上次没跑完：{}",
        text_of(&map, "detail")
    );
}

#[test]
fn interrupted_file_row_keeps_progress_and_says_interrupted() {
    let mut record = row_from(1, 0, "src", &job("a.mkv"), 1_790_000_000, 1);
    record.state = TransferState::Interrupted;
    record.bytes = 512;
    record.total = Some(1024);

    let map = record_map(&record, &[]);
    assert_eq!(text_of(&map, "stateText"), "已中断");
    assert_eq!(int_of(&map, "progress"), 50, "进度条停在历史位置");
    assert_eq!(text_of(&map, "detail"), "上次退出时未完成");
    assert_eq!(text_of(&map, "path"), "");
}

#[test]
fn clear_all_keeps_interrupted_and_active_batches() {
    let mut records = Vec::new();
    add_batch(&mut records, 1, &["done.mkv"]);
    records[0].state = TransferState::Done;

    add_batch(&mut records, 2, &["interrupted.mkv"]);
    records[1].state = TransferState::Interrupted;

    add_batch(&mut records, 3, &["running.mkv"]);
    records[2].state = TransferState::Running;

    clear_all_in(&mut records);
    assert_eq!(records.len(), 2, "只清已结束的整批");
    assert!(records.iter().all(|record| record.batch != 1));
    assert!(records.iter().any(|record| record.batch == 2), "已中断保留");
    assert!(records.iter().any(|record| record.batch == 3), "进行中保留");
}

#[test]
fn history_days_lists_newest_first_with_labels() {
    let now = 1_790_000_000;
    let day = 24 * 3600;
    let mut records = Vec::new();
    for (batch, age) in [(1u64, 0i64), (2, day), (3, 3 * day)] {
        let mut row = row_from(batch, 0, "src", &job("a.mkv"), now - age, 1);
        row.state = TransferState::Done;
        records.push(row);
    }
    let days = history_days(&records, now);
    let keys: Vec<&str> = days.iter().map(|(key, _)| key.as_str()).collect();
    assert_eq!(keys.len(), 3);
    assert_eq!(keys[0], day_key(now), "新 → 旧");
    assert_eq!(days[0].1, "今天");
    assert_eq!(days[1].1, "昨天");
    assert!(days[2].1.len() <= 5, "更早的只显示月-日：{}", days[2].1);
}

#[test]
fn history_snapshot_filters_by_day_but_keeps_active_rows() {
    let now = 1_790_000_000;
    let day = 24 * 3600;
    let mut records = Vec::new();

    let mut old_done = row_from(1, 0, "src", &job("old.mkv"), now - 3 * day, 1);
    old_done.state = TransferState::Done;
    records.push(old_done);

    let mut today_done = row_from(2, 0, "src", &job("today.mkv"), now, 1);
    today_done.state = TransferState::Done;
    records.push(today_done);

    let mut interrupted = row_from(3, 0, "src", &job("bg.mkv"), now - 5 * day, 1);
    interrupted.state = TransferState::Interrupted;
    records.push(interrupted);

    let mut running = row_from(4, 0, "src", &job("run.mkv"), now - 5 * day, 1);
    running.state = TransferState::Running;
    records.push(running);

    let (list, active) = build_history_snapshot(&records, &day_key(now));
    let names: Vec<String> = (0..list.len())
        .map(|index| text_of(&map_at(&list, index), "name"))
        .collect();
    assert!(
        names.iter().any(|name| name == "today.mkv"),
        "今天完成的可见"
    );
    assert!(
        !names.iter().any(|name| name == "old.mkv"),
        "三天前的不在「今天」"
    );
    assert!(
        names.iter().any(|name| name == "bg.mkv"),
        "已中断不受日期过滤"
    );
    assert!(
        names.iter().any(|name| name == "run.mkv"),
        "进行中不受日期过滤"
    );
    assert_eq!(active, 1);

    let (all, _) = build_history_snapshot(&records, HISTORY_DAY_ALL);
    assert_eq!(all.len(), 4, "「全部」不过滤");
}

#[test]
fn progress_stamp_tracks_running_bytes_only() {
    let mut records = Vec::new();
    add_batch(&mut records, 1, &["a.mkv", "b.mkv"]);
    let idle = progress_stamp_in(&records);
    set(&mut records, 1, 0, TransferState::Running);
    set(&mut records, 1, 1, TransferState::Running);
    assert_ne!(progress_stamp_in(&records), idle, "行开始跑本身算进度变化");

    let before = progress_stamp_in(&records);
    progress_in_at(&mut records, 1, 0, 4096, None, Instant::now());
    assert_ne!(progress_stamp_in(&records), before, "走了新字节必须变");

    // 卡住：同样的字节数不改变指纹，心跳不会反复重写整张表
    let stuck = progress_stamp_in(&records);
    progress_in_at(&mut records, 1, 0, 4096, None, Instant::now());
    assert_eq!(progress_stamp_in(&records), stuck);

    // 已结束的行不再影响指纹（结构事件自己会写盘）
    set(&mut records, 1, 0, TransferState::Done);
    let done = progress_stamp_in(&records);
    progress_in_at(&mut records, 1, 0, 999_999, None, Instant::now());
    assert_eq!(progress_stamp_in(&records), done);
}
