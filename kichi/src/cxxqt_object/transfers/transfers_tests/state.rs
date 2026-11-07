use super::super::*;
use super::*;

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
fn attention_rows_come_before_completed_rows() {
    let mut records = Vec::new();
    add_batch(&mut records, 1, &["old-interrupted"]);
    add_batch(&mut records, 2, &["new-done"]);
    add_batch(&mut records, 3, &["newest-cancelled"]);
    set(&mut records, 1, 0, TransferState::Interrupted);
    set(&mut records, 2, 0, TransferState::Done);
    set(&mut records, 3, 0, TransferState::Cancelled);

    let ordered = ordered(&records);
    let names: Vec<String> = ordered.iter().map(|record| record.display_name()).collect();
    assert_eq!(
        names,
        vec!["old-interrupted", "newest-cancelled", "new-done"],
        "待处理（失败 / 已中断）在已完成之前，组内各自新 → 旧"
    );
}

#[test]
fn paused_rows_sit_between_active_and_attention() {
    let mut records = Vec::new();
    add_batch(&mut records, 1, &["paused"]);
    add_batch(&mut records, 2, &["running"]);
    add_batch(&mut records, 3, &["failed"]);
    add_batch(&mut records, 4, &["done"]);
    set(&mut records, 1, 0, TransferState::Paused);
    set(&mut records, 2, 0, TransferState::Running);
    set(&mut records, 3, 0, TransferState::Failed);
    set(&mut records, 4, 0, TransferState::Done);

    let ordered = ordered(&records);
    let names: Vec<String> = ordered.iter().map(|record| record.display_name()).collect();
    assert_eq!(names, vec!["running", "paused", "failed", "done"]);
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
fn cancelling_one_direction_leaves_the_other_direction_untouched() {
    let mut records = Vec::new();
    records.push(row_from(
        1,
        0,
        "download",
        &job("下载.mkv"),
        1_790_000_000,
        1,
    ));
    records.push(row_from(
        2,
        0,
        "upload:/home/me/上传.mkv",
        &upload_job("上传.mkv", 2048),
        1_790_000_000,
        1,
    ));
    set(&mut records, 1, 0, TransferState::Queued);
    set(&mut records, 2, 0, TransferState::Running);

    cancel_active_for_in(&mut records, TransferDirection::Upload, "只取消上传");
    assert_eq!(
        records[0].state,
        TransferState::Queued,
        "下载不能被上传取消"
    );
    assert_eq!(records[1].state, TransferState::Cancelled, "上传应被取消");

    set(&mut records, 1, 0, TransferState::Running);
    cancel_active_for_in(&mut records, TransferDirection::Download, "只取消下载");
    assert_eq!(records[0].state, TransferState::Cancelled, "下载应被取消");
    assert_eq!(
        records[1].detail, "只取消上传",
        "后续取消下载不能覆盖上传记录"
    );
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
fn clear_finished_keeps_failed_batches_for_retry() {
    let mut records = Vec::new();
    add_batch(&mut records, 1, &["done"]);
    add_batch(&mut records, 2, &["failed"]);
    set(&mut records, 1, 0, TransferState::Done);
    set(&mut records, 2, 0, TransferState::Failed);

    clear_finished_in(&mut records);
    assert_eq!(records.len(), 1, "只清已完成，失败要留给用户重试");
    assert_eq!(records[0].batch, 2);
}

#[test]
fn cancelling_a_single_file_drops_the_whole_batch_record() {
    let mut records = Vec::new();
    add_batch(&mut records, 1, &["a"]);
    cancel_active_in(&mut records, Some(1), "");

    assert!(prune_cancelled_in(&mut records, &BTreeSet::new()));
    assert!(records.is_empty(), "取消即删记录");
}

#[test]
fn cancelling_one_child_drops_only_that_row() {
    let mut records = Vec::new();
    folder_batch(
        &mut records,
        1,
        "剧集",
        &[("a.mkv", 10), ("b.mkv", 10), ("c.mkv", 10)],
    );
    set_child(&mut records, 1, 0, TransferState::Done);
    set_child(&mut records, 1, 1, TransferState::Running);
    set_child(&mut records, 1, 2, TransferState::Queued);
    rebuild_all_batch_stats(&mut records);
    refresh_parent_state(&mut records, 1);

    cancel_file_in(&mut records, 1, 2);
    assert!(prune_cancelled_in(&mut records, &BTreeSet::new()));
    assert_eq!(records.len(), 3, "父行 + 两个未取消子行");
    assert_eq!(records[0].state, TransferState::Running, "父行仍在进行");
    assert_eq!(records[0].batch_stats.total, 2);
    assert_eq!(records[0].batch_stats.cancelled, 0);
}

#[test]
fn cancelling_a_folder_parent_drops_the_whole_batch() {
    let mut records = Vec::new();
    folder_batch(&mut records, 1, "剧集", &[("a.mkv", 10), ("b.mkv", 10)]);
    set_child(&mut records, 1, 0, TransferState::Done);
    set_child(&mut records, 1, 1, TransferState::Running);

    cancel_active_in(&mut records, Some(1), "");
    assert!(prune_cancelled_in(&mut records, &BTreeSet::new()));
    assert!(records.is_empty(), "整批取消连已完成的子行记录一起删");
}

#[test]
fn cancelled_batch_waiting_for_worker_keeps_its_rows() {
    let mut records = Vec::new();
    folder_batch(&mut records, 1, "剧集", &[("a.mkv", 10), ("b.mkv", 10)]);
    set_child(&mut records, 1, 0, TransferState::Done);
    set_child(&mut records, 1, 1, TransferState::Running);
    cancel_active_in(&mut records, Some(1), "");

    let busy = BTreeSet::from([1]);
    assert!(
        !prune_cancelled_in(&mut records, &busy),
        "线程还在收尾，记录要留给 discard_batch_parts 清断点"
    );
    assert_eq!(records.len(), 3);

    assert!(prune_cancelled_in(&mut records, &BTreeSet::new()));
    assert!(records.is_empty(), "线程退出后的下一次同步再删");
}

#[test]
fn prune_keeps_finished_batches_untouched() {
    let mut records = Vec::new();
    add_batch(&mut records, 1, &["done"]);
    set(&mut records, 1, 0, TransferState::Done);

    assert!(!prune_cancelled_in(&mut records, &BTreeSet::new()));
    assert_eq!(records.len(), 1);
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
