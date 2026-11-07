use super::super::*;
use super::*;

#[test]
fn clean_exit_marks_only_active_top_level_batches_for_auto_resume() {
    let mut records = Vec::new();
    folder_batch(
        &mut records,
        1,
        "剧集",
        &[("running.mkv", 10), ("paused.mkv", 10), ("done.mkv", 10)],
    );
    records[0].state = TransferState::Running;
    set_child(&mut records, 1, 0, TransferState::Running);
    set_child(&mut records, 1, 1, TransferState::Paused);
    set_child(&mut records, 1, 2, TransferState::Done);
    mark_auto_resume_in(&mut records);
    assert!(records[0].auto_resume, "运行中的文件夹总账行要续跑");
    assert!(records[1].auto_resume, "运行中的子文件要续跑");
    assert!(!records[2].auto_resume, "暂停子行不自动续跑");
    assert!(!records[3].auto_resume, "已完成子行不自动续跑");

    records[0].state = TransferState::Pausing;
    mark_auto_resume_in(&mut records);
    assert!(
        records.iter().all(|record| !record.auto_resume),
        "正在暂停的任务按用户意图保持暂停，不能在下次启动自动跑起来"
    );
}

#[test]
fn clean_exit_marks_only_active_upload_rows() {
    let mut records = Vec::new();
    let parent_job = TransferJob {
        id: "upload-folder:/home/me/剧集".to_owned(),
        name: "剧集".to_owned(),
        kind: EntryKind::Folder,
        size: 0,
        audit_flagged: false,
        sub_dir: Vec::new(),
        dir: PathBuf::new(),
        direction: TransferDirection::Upload,
        upload: Some(UploadJob {
            local_path: PathBuf::from("/home/me/剧集"),
            parent_id: "cloud-folder".to_owned(),
            parent_name: "动画".to_owned(),
            ..UploadJob::default()
        }),
    };
    let mut parent = row_from(1, 0, &parent_job.id, &parent_job, 1, 1);
    parent.state = TransferState::Running;
    records.push(parent);
    let seeds: Vec<TransferJob> = [
        ("running.mkv", TransferState::Running),
        ("queued.mkv", TransferState::Queued),
        ("paused.mkv", TransferState::Paused),
        ("failed.mkv", TransferState::Failed),
    ]
    .into_iter()
    .map(|(name, _)| {
        let mut job = upload_job(name, 10);
        job.sub_dir = vec!["剧集".to_owned()];
        job
    })
    .collect();
    expand_folder_in(&mut records, 1, &seeds, 1, 1);
    for (index, state) in [
        TransferState::Running,
        TransferState::Queued,
        TransferState::Paused,
        TransferState::Failed,
    ]
    .into_iter()
    .enumerate()
    {
        child_mut(&mut records, 1, index).state = state;
    }
    rebuild_batch_stats(&mut records, 1);

    let mut paused = row_from(
        2,
        0,
        "upload:/home/me/paused.mkv",
        &upload_job("paused.mkv", 10),
        1,
        1,
    );
    paused.state = TransferState::Paused;
    records.push(paused);

    mark_auto_resume_uploads_in(&mut records);

    assert!(records[0].auto_resume, "在途文件夹总账行要续传");
    assert!(records[1].auto_resume, "运行中的子文件要续传");
    assert!(records[2].auto_resume, "排队中的子文件要续传");
    assert!(!records[3].auto_resume, "暂停子文件不自动续传");
    assert!(!records[4].auto_resume, "失败子文件不自动续传");
    assert!(!records[5].auto_resume, "暂停批次不自动续传");
}

#[test]
fn auto_resume_folder_requeues_only_the_selected_interrupted_children() {
    let mut records = Vec::new();
    let parent_job = TransferJob {
        id: "upload-folder:/home/me/剧集".to_owned(),
        name: "剧集".to_owned(),
        kind: EntryKind::Folder,
        size: 0,
        audit_flagged: false,
        sub_dir: Vec::new(),
        dir: PathBuf::new(),
        direction: TransferDirection::Upload,
        upload: Some(UploadJob {
            local_path: PathBuf::from("/home/me/剧集"),
            parent_id: "cloud-folder".to_owned(),
            parent_name: "动画".to_owned(),
            ..UploadJob::default()
        }),
    };
    let mut parent = row_from(1, 0, &parent_job.id, &parent_job, 1, 1);
    parent.state = TransferState::Interrupted;
    records.push(parent);
    let seeds: Vec<TransferJob> = ["a.mkv", "b.mkv", "done.mkv", "failed.mkv"]
        .into_iter()
        .map(|name| {
            let mut job = upload_job(name, 10);
            job.sub_dir = vec!["剧集".to_owned()];
            job
        })
        .collect();
    expand_folder_in(&mut records, 1, &seeds, 1, 1);
    for (index, state) in [
        TransferState::Interrupted,
        TransferState::Interrupted,
        TransferState::Done,
        TransferState::Failed,
    ]
    .into_iter()
    .enumerate()
    {
        child_mut(&mut records, 1, index).state = state;
    }
    rebuild_batch_stats(&mut records, 1);

    assert!(resume_auto_upload_folder_in(&mut records, 1, &[0]));
    assert_eq!(records[0].state, TransferState::Queued);
    assert_eq!(child_mut(&mut records, 1, 0).state, TransferState::Queued);
    assert_eq!(
        child_mut(&mut records, 1, 1).state,
        TransferState::Interrupted,
        "没标过的旧中断项不能被顺手恢复"
    );
    assert_eq!(child_mut(&mut records, 1, 2).state, TransferState::Done);
    assert_eq!(child_mut(&mut records, 1, 3).state, TransferState::Failed);
    assert_eq!(records[0].batch_stats, batch_stats(&records, 1));

    assert!(resume_auto_upload_folder_in(&mut records, 1, &[1]));
    assert_eq!(
        child_mut(&mut records, 1, 1).state,
        TransferState::Queued,
        "第二次只恢复显式选中的兄弟文件"
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

#[test]
fn retry_upload_folder_keeps_dirs_and_successful_children() {
    let batch = 9_800_001u64;
    let parent_job = TransferJob {
        id: "upload-folder:/home/me/剧集".to_owned(),
        name: "剧集".to_owned(),
        kind: EntryKind::Folder,
        size: 0,
        audit_flagged: false,
        sub_dir: Vec::new(),
        dir: PathBuf::new(),
        direction: TransferDirection::Upload,
        upload: Some(UploadJob {
            local_path: PathBuf::from("/home/me/剧集"),
            parent_id: "cloud-target".to_owned(),
            parent_name: "动画".to_owned(),
            batch_root_id: "root-id".to_owned(),
            cloud_parent_id: String::new(),
        }),
    };
    let done_job = TransferJob {
        id: "upload:/home/me/剧集/done.mkv".to_owned(),
        name: "done.mkv".to_owned(),
        kind: EntryKind::File,
        size: 10,
        audit_flagged: false,
        sub_dir: vec!["剧集".to_owned()],
        dir: PathBuf::new(),
        direction: TransferDirection::Upload,
        upload: Some(UploadJob {
            local_path: PathBuf::from("/home/me/剧集/done.mkv"),
            parent_id: "cloud-target".to_owned(),
            parent_name: "动画".to_owned(),
            batch_root_id: "root-id".to_owned(),
            cloud_parent_id: "root-id".to_owned(),
        }),
    };
    let failed_job = TransferJob {
        id: "upload:/home/me/剧集/failed.mkv".to_owned(),
        name: "failed.mkv".to_owned(),
        kind: EntryKind::File,
        size: 10,
        audit_flagged: false,
        sub_dir: vec!["剧集".to_owned(), "失败目录".to_owned()],
        dir: PathBuf::new(),
        direction: TransferDirection::Upload,
        upload: Some(UploadJob {
            local_path: PathBuf::from("/home/me/剧集/失败目录/failed.mkv"),
            parent_id: "cloud-target".to_owned(),
            parent_name: "动画".to_owned(),
            batch_root_id: "root-id".to_owned(),
            cloud_parent_id: String::new(),
        }),
    };

    with_records(|records| {
        let mut parent = row_from(batch, 0, &parent_job.id, &parent_job, 1_790_000_000, 1);
        parent.state = TransferState::Failed;
        parent.upload_dirs = vec![UploadDir {
            path: vec!["剧集".to_owned()],
            id: "root-id".to_owned(),
        }];
        parent.upload_failed_dirs = vec![vec!["剧集".to_owned(), "失败目录".to_owned()]];
        records.push(parent);

        let mut done = row_from(batch, 0, &parent_job.id, &done_job, 1_790_000_000, 1);
        done.child = true;
        done.state = TransferState::Done;
        records.push(done);

        let mut failed = row_from(batch, 1, &parent_job.id, &failed_job, 1_790_000_000, 1);
        failed.child = true;
        failed.state = TransferState::Failed;
        records.push(failed);
    });

    assert!(retry_upload_folder(batch));
    with_records(|records| {
        let parent = records
            .iter()
            .find(|record| record.batch == batch && is_folder_ledger(record))
            .expect("父行还在");
        assert_eq!(parent.state, TransferState::Queued);
        assert_eq!(parent.upload_dirs.len(), 1, "已成功目录不能丢");
        assert!(parent.upload_failed_dirs.is_empty());
        assert_eq!(
            find_file(records, batch, 0).map(|row| row.state),
            Some(TransferState::Done),
            "成功文件保持完成，不重新上传"
        );
        assert_eq!(
            find_file(records, batch, 1).map(|row| row.state),
            Some(TransferState::Queued)
        );
    });
    with_records(|records| records.retain(|record| record.batch != batch));
}
