use super::super::*;
use super::*;

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
    assert_eq!(list.len(), 2, "统一列表只有父行 + 单文件批次");
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
fn cancelling_one_child_keeps_siblings_downloading() {
    let mut records = Vec::new();
    folder_batch(
        &mut records,
        1,
        "剧集",
        &[("a.mkv", 10), ("b.mkv", 20), ("c.mkv", 30)],
    );
    begin_in(&mut records, 1, 0);

    cancel_file_in(&mut records, 1, 0);
    assert_eq!(
        child_mut(&mut records, 1, 0).state,
        TransferState::Cancelled
    );
    assert_eq!(child_mut(&mut records, 1, 1).state, TransferState::Queued);
    assert_eq!(child_mut(&mut records, 1, 2).state, TransferState::Queued);
    assert_eq!(
        records[0].state,
        TransferState::Running,
        "同批还有排队子项，父行继续显示下载中"
    );

    begin_in(&mut records, 1, 1);
    finish_ok_in(&mut records, 1, 1, Path::new("/tmp/b.mkv"));
    begin_in(&mut records, 1, 2);
    finish_ok_in(&mut records, 1, 2, Path::new("/tmp/c.mkv"));
    assert_eq!(
        records[0].state,
        TransferState::Cancelled,
        "其余子项完成后，父行如实保留部分取消状态"
    );

    let stats = batch_stats(&records, 1);
    assert_eq!((stats.done, stats.cancelled, stats.failed), (2, 1, 0));
    let map = record_map(&records[0], &records);
    assert_eq!(text_of(&map, "detail"), "完成 2 个 · 取消 1 个");
}

#[test]
fn cancelling_a_finished_child_does_not_rewrite_it() {
    let mut records = Vec::new();
    folder_batch(&mut records, 1, "剧集", &[("a.mkv", 10)]);
    finish_ok_in(&mut records, 1, 0, Path::new("/tmp/a.mkv"));

    cancel_file_in(&mut records, 1, 0);
    assert_eq!(child_mut(&mut records, 1, 0).state, TransferState::Done);
    assert_eq!(records[0].state, TransferState::Done);
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
    child_mut(&mut records, 1, 0).bytes = 10;
    set_child(&mut records, 1, 0, TransferState::Done);
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
fn incremental_batch_cache_matches_a_full_recount() {
    let mut records = Vec::new();
    folder_batch(
        &mut records,
        1,
        "剧集",
        &[("a.mkv", 10), ("b.mkv", 20), ("c.mkv", 30)],
    );

    begin_in(&mut records, 1, 0);
    progress_in(&mut records, 1, 0, 4, Some(10));
    finish_ok_in(&mut records, 1, 0, Path::new("/tmp/a.mkv"));
    begin_in(&mut records, 1, 1);
    progress_in(&mut records, 1, 1, 5, Some(20));
    finish_err_in(&mut records, 1, 2, "坏链");

    assert_eq!(
        records[0].batch_stats,
        batch_stats(&records, 1),
        "增量缓存必须与整批重算一致"
    );
    assert_eq!(records[0].batch_stats.running_position, Some(2));
}

#[test]
fn running_position_uses_the_rank_not_a_sparse_child_index() {
    let mut records = Vec::new();
    folder_batch(
        &mut records,
        1,
        "剧集",
        &[("a.mkv", 10), ("b.mkv", 20), ("c.mkv", 30)],
    );

    // 子文件重试后 index 可能稀疏：index 0 已不在记录表里，剩 1、2。
    let first = child_at(&records, 1, 0);
    records.remove(first);
    rebuild_batch_stats(&mut records, 1);
    begin_in(&mut records, 1, 2);

    assert_eq!(
        records[0].batch_stats.running_position,
        Some(2),
        "第 2 个（index=2）不是第 3 个"
    );
    assert_eq!(records[0].batch_stats, batch_stats(&records, 1));
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
    assert_eq!(text_of(&map, "detail"), "完成 1 个 · 取消 1 个");
}
