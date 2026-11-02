//! `transfers_history.rs` 的单元测试：读写的往返 / 损坏文件 / 高版本 / 字段缺失 /
//! 恢复规则 / 清理规则 / 节流判定。
//!
//! 全部用临时目录（每个用例自己的文件名），不碰 `~/.config/kichi`。

use std::path::{Path, PathBuf};

use kichi_core::EntryKind;

use super::*;
use crate::cxxqt_object::transfers::{row_from, with_trees, TransferJob, TransferState};

/// 每个用例一个独立的临时文件路径（不依赖外部 crate，只用 pid + 名字）。
fn temp_path(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("kichi-history-{}-{name}.json", std::process::id()));
    let _ = std::fs::remove_file(&path);
    path
}

fn job(name: &str, kind: EntryKind) -> TransferJob {
    TransferJob {
        id: format!("id-{name}"),
        name: name.to_owned(),
        kind,
        size: 1024,
        audit_flagged: false,
        sub_dir: Vec::new(),
        dir: PathBuf::from("/home/me/Downloads"),
        direction: TransferDirection::Download,
        upload: None,
    }
}

fn file_row(batch: u64, name: &str) -> crate::cxxqt_object::transfers::Transfer {
    let mut row = row_from(
        batch,
        0,
        &format!("src-{name}"),
        &job(name, EntryKind::File),
        1_790_000_000,
        7,
    );
    row.state = TransferState::Done;
    row.path = Some(PathBuf::from(format!("/home/me/Downloads/{name}")));
    row
}

fn upload_row(batch: u64, name: &str) -> crate::cxxqt_object::transfers::Transfer {
    let upload_job = TransferJob {
        id: format!("upload:/home/me/{name}"),
        name: name.to_owned(),
        kind: EntryKind::File,
        size: 2048,
        audit_flagged: false,
        sub_dir: Vec::new(),
        dir: PathBuf::new(),
        direction: TransferDirection::Upload,
        upload: Some(UploadJob {
            local_path: PathBuf::from(format!("/home/me/{name}")),
            parent_id: "parent-42".to_owned(),
            parent_name: "动画".to_owned(),
        }),
    };
    let mut row = row_from(batch, 0, &upload_job.id, &upload_job, 1_790_000_000, 7);
    row.state = TransferState::Failed;
    row.bytes = 1024;
    row.detail = "上传连接中断".to_owned();
    row
}

fn folder_rows(batch: u64, name: &str) -> Vec<crate::cxxqt_object::transfers::Transfer> {
    let folder_job = job(name, EntryKind::Folder);
    let mut parent = restore_row(
        batch,
        0,
        false,
        &folder_job,
        &format!("src-{name}"),
        TransferState::Running,
        0,
        None,
        String::new(),
        None,
        false,
        0,
        1_790_000_000,
        7,
        false,
    );
    parent.source_id = format!("src-{name}");
    let child_job = TransferJob {
        id: format!("id-{name}-a"),
        name: "a.mkv".to_owned(),
        kind: EntryKind::File,
        sub_dir: vec![name.to_owned()],
        ..job("a.mkv", EntryKind::File)
    };
    let child = restore_row(
        batch,
        0,
        true,
        &child_job,
        &format!("src-{name}"),
        TransferState::Running,
        512,
        Some(1024),
        String::new(),
        None,
        false,
        0,
        1_790_000_000,
        7,
        false,
    );
    parent.skipped = 0;
    vec![parent, child]
}

fn entries(
    rows: &[crate::cxxqt_object::transfers::Transfer],
    dirs: &[(u64, Vec<Vec<String>>)],
) -> Vec<EntryRecord> {
    entries_of(rows, dirs)
}

#[test]
fn round_trip_keeps_every_recoverable_field() {
    let path = temp_path("round-trip");
    let mut done = file_row(1, "a.mkv");
    done.skipped = 2;

    let mut cancelled = file_row(2, "b.mkv");
    cancelled.state = TransferState::Cancelled;
    cancelled.detail = "用户取消".to_owned();
    cancelled.path = None;

    let rows = vec![done.clone(), cancelled.clone()];
    let written = entries(&rows, &[]);
    save(&path, &written).expect("写盘成功");

    let loaded = load(&path);
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0].job.name, "a.mkv");
    assert_eq!(loaded[0].state, "done");
    assert_eq!(loaded[0].skipped, 2);
    assert_eq!(loaded[0].created_unix, 1_790_000_000);
    assert_eq!(loaded[0].session, 7);
    assert_eq!(loaded[1].state, "cancelled");
    assert_eq!(loaded[1].detail, "用户取消");
    assert_eq!(loaded[1].path, None);

    let restored = restore(loaded, 8);
    assert_eq!(restored.len(), 2);
    let done = restored
        .iter()
        .find(|row| row.job.name == "a.mkv")
        .expect("完成的文件还在");
    assert_eq!(done.state, TransferState::Done);
    assert_eq!(done.path, Some(PathBuf::from("/home/me/Downloads/a.mkv")));
    assert_eq!(done.session, 8, "读回的行盖上这一轮的会话号");

    // 文件权限：只有本人可读写
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path)
            .expect("文件存在")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn corrupt_file_loads_empty_and_does_not_panic() {
    let path = temp_path("corrupt");
    std::fs::write(&path, b"{ not json at all").expect("写坏文件");
    assert!(load(&path).is_empty());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn missing_file_loads_empty() {
    let path = temp_path("missing");
    assert!(load(&path).is_empty());
}

#[test]
fn higher_version_is_parsed_with_known_fields_only() {
    let path = temp_path("future");
    let body = r#"{
        "version": 99,
        "something_new": true,
        "entries": [
            {
                "batch": 3, "index": 0, "child": false, "source_id": "src",
                "job": { "id": "x", "name": "x.mkv", "kind": "file", "dir": "/tmp" },
                "state": "done", "path": "/tmp/x.mkv",
                "future_field": { "nested": [1, 2, 3] }
            }
        ]
    }"#;
    std::fs::write(&path, body).expect("写文件");
    let loaded = load(&path);
    assert_eq!(loaded.len(), 1, "高版本号照常按已知字段解析");
    assert_eq!(loaded[0].state, "done");
    let restored = restore(loaded, 2);
    assert_eq!(restored[0].state, TransferState::Done);
    assert_eq!(
        restored[0].job.direction,
        TransferDirection::Download,
        "旧记录没有 direction 字段时仍按下载处理"
    );
    assert!(restored[0].job.upload.is_none());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn v1_download_record_without_upload_fields_stays_a_download() {
    let path = temp_path("v1-download");
    std::fs::write(
        &path,
        r#"{
            "version": 1,
            "entries": [{
                "batch": 1, "index": 0, "child": false,
                "source_id": "file-1",
                "job": {
                    "id": "file-1", "name": "旧下载.mkv", "kind": "file",
                    "size": 1024, "dir": "/home/me/Downloads"
                },
                "state": "done",
                "path": "/home/me/Downloads/旧下载.mkv"
            }]
        }"#,
    )
    .expect("写 v1 历史");

    let restored = restore(load(&path), 2);
    assert_eq!(restored.len(), 1);
    assert_eq!(restored[0].job.direction, TransferDirection::Download);
    assert!(restored[0].job.upload.is_none());
    assert_eq!(restored[0].state, TransferState::Done);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn upload_record_round_trips_with_its_retry_target() {
    let path = temp_path("upload-round-trip");
    let row = upload_row(6, "电影.mkv");
    save(&path, &entries(&[row], &[])).expect("写上传历史");

    let loaded = load(&path);
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].job.direction, "upload");
    assert_eq!(loaded[0].job.local_path, "/home/me/电影.mkv");
    assert_eq!(loaded[0].job.parent_id, "parent-42");
    assert_eq!(loaded[0].job.parent_name, "动画");

    let restored = restore(loaded, 8);
    assert_eq!(restored.len(), 1);
    let job = &restored[0].job;
    assert_eq!(job.direction, TransferDirection::Upload);
    let upload = job.upload.as_ref().expect("上传重试目标读回");
    assert_eq!(upload.local_path, PathBuf::from("/home/me/电影.mkv"));
    assert_eq!(upload.parent_id, "parent-42");
    assert_eq!(upload.parent_name, "动画");
    assert_eq!(restored[0].state, TransferState::Failed);
    assert_eq!(restored[0].detail, "上传连接中断");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn missing_fields_fall_back_to_defaults() {
    let path = temp_path("sparse");
    std::fs::write(&path, r#"{"entries":[{"batch":4}]}"#).expect("写文件");
    let loaded = load(&path);
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].batch, 4);
    assert_eq!(loaded[0].job.name, "");
    assert_eq!(loaded[0].bytes, 0);
    assert_eq!(loaded[0].created_unix, 0);
    // 缺状态 = 认不出 = 已中断，且不 panic
    let restored = restore(loaded, 1);
    assert_eq!(restored[0].state, TransferState::Interrupted);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn active_rows_become_interrupted() {
    for state in [
        TransferState::Queued,
        TransferState::Preparing,
        TransferState::Running,
    ] {
        let mut row = file_row(1, "x.mkv");
        row.state = state;
        row.bytes = 300;
        row.total = Some(1024);
        let restored = restore(entries(&[row], &[]), 2);
        assert_eq!(restored[0].state, TransferState::Interrupted);
        assert_eq!(restored[0].bytes, 300, "已下字节保留，进度条停在历史位置");
        assert_eq!(restored[0].total, Some(1024));
        assert!(!restored[0].state.is_active(), "已中断不占调度槽");
        assert!(!restored[0].state.is_finished(), "已中断不算已结束");
    }
}

#[test]
fn pausing_or_paused_rows_restore_as_paused() {
    for state in [TransferState::Pausing, TransferState::Paused] {
        let mut row = file_row(1, "paused.mkv");
        row.state = state;
        let restored = restore(entries(&[row], &[]), 2);
        assert_eq!(
            restored[0].state,
            TransferState::Paused,
            "{state:?} 重启后要保留用户暂停意图，让用户手动继续"
        );
        assert!(!restored[0].state.is_active());
        assert!(restored[0].state.is_cancellable());
    }
}

#[test]
fn auto_resume_marker_survives_roundtrip_until_consumed() {
    let mut row = file_row(1, "resume.mkv");
    row.state = TransferState::Running;
    row.auto_resume = true;

    let restored = restore(entries(&[row], &[]), 2);
    assert_eq!(restored[0].state, TransferState::Interrupted);
    assert!(restored[0].auto_resume, "读回后仍要知道这一行需要自动续跑");
    let path = temp_path("auto-resume-marker");
    save(&path, &entries(&restored, &[])).expect("保存续跑标记");
    let loaded = load(&path);
    assert!(loaded[0].auto_resume, "标记必须真的写进 JSON");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn finished_rows_keep_their_state() {
    for state in [
        TransferState::Done,
        TransferState::Failed,
        TransferState::Cancelled,
    ] {
        let mut row = file_row(1, "x.mkv");
        row.state = state;
        row.detail = "原因".to_owned();
        let restored = restore(entries(&[row], &[]), 2);
        assert_eq!(restored[0].state, state);
        assert_eq!(restored[0].detail, "原因");
    }
}

#[test]
fn session_counter_continues_after_previous_max() {
    assert_eq!(next_session(0), 1);
    assert_eq!(next_session(41), 42);
    assert_eq!(next_session(u64::MAX), u64::MAX);
}

#[test]
fn orphan_batch_without_parent_is_dropped() {
    let rows = folder_rows(1, "剧集");
    let only_child = vec![rows[1].clone()];
    let restored = restore(entries(&only_child, &[]), 2);
    assert!(restored.is_empty(), "父行不存在：整批丢弃");
}

#[test]
fn child_outside_its_folder_tree_is_dropped() {
    let rows = folder_rows(1, "剧集");
    // 子行写在别的目录树下：父行与子行绑不上
    let dirs = vec![(1u64, vec![vec!["别的文件夹".to_owned()]])];
    let restored = restore(entries(&rows, &dirs), 2);
    assert!(restored.is_empty(), "绑不上：整批丢弃");
}

#[test]
fn consistent_folder_batch_restores_parent_first_then_children() {
    let rows = folder_rows(1, "剧集");
    let dirs = vec![(1u64, vec![vec!["剧集".to_owned()]])];
    let restored = restore(entries(&rows, &dirs), 2);
    assert_eq!(restored.len(), 2);
    assert!(!restored[0].child, "父行在最前");
    assert!(restored[1].child);
    assert_eq!(restored[0].state, TransferState::Interrupted);
    assert_eq!(restored[1].state, TransferState::Interrupted);
    assert_eq!(restored[1].job.sub_dir, vec!["剧集".to_owned()]);
}

#[test]
fn single_file_batch_with_children_is_dropped() {
    let mut parent = file_row(1, "a.mkv");
    parent.state = TransferState::Done;
    let mut stray = file_row(1, "b.mkv");
    stray.child = true;
    stray.state = TransferState::Done;
    let restored = restore(entries(&[parent, stray], &[]), 2);
    assert!(restored.is_empty(), "单文件批次不该有子行");
}

#[test]
fn expired_batches_skip_done_and_keep_recent() {
    let now = 1_800_000_000;
    let day = 24 * 3600;
    let mut old_failed = file_row(1, "old-failed.mkv");
    old_failed.state = TransferState::Failed;
    old_failed.created_unix = now - 100 * day;

    let mut old_done = file_row(2, "old-done.mkv");
    old_done.state = TransferState::Done;
    old_done.created_unix = now - 100 * day;

    let mut recent_failed = file_row(3, "recent.mkv");
    recent_failed.state = TransferState::Failed;
    recent_failed.created_unix = now - 10 * day;

    let records = vec![old_failed, old_done, recent_failed];
    let expired = expired_batches(&records, now, RETENTION_DAYS);
    assert_eq!(expired, vec![1], "只删超期且没有已完成行的整批");
}

#[test]
fn expired_batches_ignore_rows_without_timestamp() {
    let now = 1_800_000_000;
    let mut row = file_row(1, "legacy.mkv");
    row.state = TransferState::Failed;
    row.created_unix = 0;
    assert!(expired_batches(&[row], now, RETENTION_DAYS).is_empty());
}

#[test]
fn expired_batches_keep_paused_work_even_when_old() {
    let now = 1_800_000_000;
    let day = 24 * 3600;
    let mut paused = folder_rows(1, "暂停的剧集");
    paused[0].state = TransferState::Paused;
    paused[1].state = TransferState::Paused;
    paused[0].created_unix = now - 100 * day;
    paused[1].created_unix = now - 100 * day;

    let mut old_failed = file_row(2, "old-failed.mkv");
    old_failed.state = TransferState::Failed;
    old_failed.created_unix = now - 100 * day;

    let expired = expired_batches(
        &[paused[0].clone(), paused[1].clone(), old_failed],
        now,
        RETENTION_DAYS,
    );
    assert_eq!(expired, vec![2], "暂停批次无论多久都要留给用户继续");
}

#[test]
fn oversized_history_drops_oldest_finished_batches_first() {
    let mut rows = Vec::new();
    for batch in 1..=6u64 {
        let mut row = file_row(batch, &format!("big-{batch}.mkv"));
        row.job.name = "x".repeat(2000);
        rows.push(row);
    }
    // 最新一批还在下载：不该被丢
    rows[5].state = TransferState::Running;

    let written = entries(&rows, &[]);
    let refs: Vec<&EntryRecord> = written.iter().collect();
    let total = estimate_size(written.iter());
    let dropped = oversized_batches(&refs, total / 2, total / 4);
    assert!(dropped.contains(&1), "最旧的先丢");
    assert!(!dropped.contains(&6), "进行中的批次绝不丢");
    assert!(dropped.windows(2).all(|pair| pair[0] < pair[1]), "从旧到新");
}

#[test]
fn oversized_history_keeps_the_last_batch() {
    let mut rows = Vec::new();
    for batch in 1..=2u64 {
        let mut row = file_row(batch, &format!("big-{batch}.mkv"));
        row.job.name = "x".repeat(4000);
        rows.push(row);
    }
    let written = entries(&rows, &[]);
    let refs: Vec<&EntryRecord> = written.iter().collect();
    let dropped = oversized_batches(&refs, 1, 1);
    assert_eq!(dropped, vec![1], "留一个批次，不把表清空");
}

#[test]
fn small_history_is_not_trimmed() {
    let written = entries(&[file_row(1, "a.mkv")], &[]);
    let refs: Vec<&EntryRecord> = written.iter().collect();
    assert!(oversized_batches(&refs, MAX_FILE_BYTES, SHRINK_TO_BYTES).is_empty());
}

#[test]
fn write_is_throttled_to_two_seconds() {
    use std::time::{Duration, Instant};
    let start = Instant::now();
    assert!(write_due(None, start), "第一次写盘不等");
    assert!(!write_due(Some(start), start + Duration::from_millis(1999)));
    assert!(write_due(Some(start), start + WRITE_INTERVAL));
    assert!(write_due(Some(start), start + Duration::from_secs(3)));
}

#[test]
fn progress_write_needs_both_the_interval_and_new_bytes() {
    use std::time::{Duration, Instant};
    let start = Instant::now();
    // 第一次：没有上次写盘记录，字节也算「变了」
    assert!(progress_write_due(None, start, None, 100));
    // 卡住：指纹没变，即使节流到点也不重写
    assert!(!progress_write_due(
        None,
        start + Duration::from_secs(5),
        Some(100),
        100
    ));
    // 走到新字节但要等节流
    assert!(!progress_write_due(
        Some(start),
        start + Duration::from_millis(1999),
        Some(100),
        200
    ));
    assert!(progress_write_due(
        Some(start),
        start + WRITE_INTERVAL,
        Some(100),
        200
    ));
}

#[test]
fn folder_dirs_are_written_on_the_ledger_only() {
    let rows = folder_rows(1, "剧集");
    let dirs = vec![(
        1u64,
        vec![
            vec!["剧集".to_owned()],
            vec!["剧集".to_owned(), "第一季".to_owned()],
        ],
    )];
    let written = entries(&rows, &dirs);
    let ledger = written.iter().find(|entry| !entry.child).expect("有总账行");
    assert_eq!(ledger.dirs.len(), 2, "总账行带目录树");
    let child = written.iter().find(|entry| entry.child).expect("有子行");
    assert!(child.dirs.is_empty(), "同批的子行不带（只写一份）");
}

#[test]
fn folder_tree_survives_the_round_trip() {
    let rows = folder_rows(1, "剧集");
    let dirs = vec![(
        1u64,
        vec![
            vec!["剧集".to_owned()],
            vec!["剧集".to_owned(), "第一季".to_owned()],
        ],
    )];
    let written = entries(&rows, &dirs);
    assert_eq!(trees_of(&written), dirs, "从总账行读回同一份目录树");
}

#[test]
fn write_now_stores_the_folder_tree_once() {
    // 真实路径：内存记录表 + 目录树 → 文件。批次号取一个其它用例不会用到的值
    let batch = 9_500_201;
    let path = temp_path("tree-once");
    let tree = vec![
        vec!["剧集".to_owned()],
        vec!["剧集".to_owned(), "第一季".to_owned()],
    ];
    with_records(|records| records.extend(folder_rows(batch, "剧集")));
    with_trees(|trees| {
        trees.insert(batch, tree.clone());
    });
    let target = StoreTarget {
        path: path.clone(),
        session: 3,
    };
    write_now(&target).expect("写盘成功");

    let loaded = load(&path);
    let mine: Vec<&EntryRecord> = loaded.iter().filter(|entry| entry.batch == batch).collect();
    assert_eq!(mine.len(), 2, "总账行 + 子行都写进去了");
    let ledger = mine.iter().find(|entry| !entry.child).expect("有总账行");
    assert_eq!(ledger.dirs, tree, "总账行带完整目录树");
    let child = mine.iter().find(|entry| entry.child).expect("有子行");
    assert!(child.dirs.is_empty(), "子行不再克隆一份");

    with_records(|records| records.retain(|record| record.batch != batch));
    with_trees(|trees| {
        trees.remove(&batch);
    });
    let _ = std::fs::remove_file(&path);
}

#[test]
fn restore_trees_keeps_only_batches_in_the_table() {
    let live = 9_500_301u64;
    let gone = 9_500_302u64;
    let tree = vec![vec!["剧集".to_owned()]];
    with_records(|records| {
        records.extend(folder_rows(live, "剧集"));
        // 调用方持有 records 锁（boot 就是这么调的）
        crate::cxxqt_object::transfers::restore_trees(
            vec![(live, tree.clone()), (gone, tree.clone())],
            records,
        );
    });
    with_trees(|trees| {
        assert_eq!(trees.get(&live), Some(&tree), "读回的批次挂上目录树");
        assert!(!trees.contains_key(&gone), "记录表里没有的批次不挂");
        trees.remove(&live);
    });
    with_records(|records| records.retain(|record| record.batch != live));
}

#[test]
fn prune_trees_drops_batches_that_left_the_record_table() {
    let live = 9_500_101u64;
    let gone = 9_500_102u64;
    with_records(|records| {
        records.push(restore_row(
            live,
            0,
            false,
            &job("留着.mkv", EntryKind::File),
            "src-live",
            TransferState::Done,
            1,
            Some(1),
            String::new(),
            None,
            false,
            0,
            1_790_000_000,
            3,
            false,
        ))
    });
    with_trees(|trees| {
        trees.insert(live, vec![vec!["留着".to_owned()]]);
        trees.insert(gone, vec![vec!["丢掉".to_owned()]]);
    });

    crate::cxxqt_object::transfers::prune_trees();

    with_trees(|trees| {
        assert!(trees.contains_key(&live), "还在记录表里的批次留着");
        assert!(!trees.contains_key(&gone), "记录表里没有的批次收走");
        trees.remove(&live);
    });
    with_records(|records| records.retain(|record| record.batch != live));
}

#[test]
fn save_replaces_the_file_atomically() {
    let path = temp_path("atomic");
    save(&path, &entries(&[file_row(1, "a.mkv")], &[])).expect("第一次写");
    save(
        &path,
        &entries(&[file_row(1, "a.mkv"), file_row(2, "b.mkv")], &[]),
    )
    .expect("第二次写");
    assert_eq!(load(&path).len(), 2);
    let tmp = path.with_extension("json.tmp");
    assert!(!Path::new(&tmp).exists(), "临时文件写完就不在了");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn write_now_mirrors_the_record_table() {
    // 真实路径：记录表 → 文件。批次号取一个其它用例不会用到的值
    let batch = 9_500_001;
    let path = temp_path("write-now");
    with_records(|records| {
        records.push(restore_row(
            batch,
            0,
            false,
            &job("镜像.mkv", EntryKind::File),
            "src-mirror",
            TransferState::Done,
            1024,
            Some(1024),
            String::new(),
            Some(PathBuf::from("/tmp/镜像.mkv")),
            false,
            0,
            1_790_000_000,
            3,
            false,
        ))
    });
    let target = StoreTarget {
        path: path.clone(),
        session: 3,
    };
    write_now(&target).expect("写盘成功");

    let loaded = load(&path);
    let row = loaded
        .iter()
        .find(|entry| entry.batch == batch)
        .expect("内存里的行出现在文件里");
    assert_eq!(row.state, "done");
    assert_eq!(row.job.name, "镜像.mkv");
    assert_eq!(row.session, 3);

    with_records(|records| records.retain(|record| record.batch != batch));
    let _ = std::fs::remove_file(&path);
}
