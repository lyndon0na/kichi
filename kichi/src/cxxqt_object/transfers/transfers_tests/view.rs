use super::super::*;
use super::*;

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

// —— 准备中 / 文件夹展开 / 父行聚合（2026-10-27 文件夹批次）——

#[test]
fn main_snapshot_counts_a_folder_as_one_row() {
    let mut records = Vec::new();
    folder_batch(&mut records, 1, "剧集", &[("a.mkv", 10), ("b.mkv", 10)]);
    add_batch(&mut records, 2, &["single.mkv"]);
    begin_in(&mut records, 2, 0);
    // 父行也随子行进入进行中
    begin_in(&mut records, 1, 0);

    let (list, active) = build_snapshot(&records);
    assert_eq!(list.len(), 2, "子行只有展开时才进入统一列表");
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

    // 第一个子行完成后仍然留在第一位（内联子行不按状态重排）
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
    child_mut(&mut records, 1, 0).bytes = 10;
    set_child(&mut records, 1, 0, TransferState::Done);
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
