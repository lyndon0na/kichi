use super::super::*;
use super::*;

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
        "统一列表的单文件行仍用完整显示路径"
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
    assert_eq!(detail, "完成 1 个 · 跳过 3 个目录");

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

// —— 已中断 / 清空全部（2026-10-28 传输历史落盘）——

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
fn paused_rows_are_actionable_but_not_active() {
    for state in [
        TransferState::Queued,
        TransferState::Preparing,
        TransferState::Running,
        TransferState::Pausing,
    ] {
        assert!(state.is_active(), "{state:?} 仍占调度槽");
        assert!(state.is_cancellable());
    }
    assert!(
        !TransferState::Paused.is_active(),
        "已暂停不占调度槽、不计入侧栏传输中"
    );
    assert!(TransferState::Paused.is_cancellable(), "暂停任务仍可取消");
    assert!(!TransferState::Paused.is_finished(), "暂停不是历史结束状态");
}
