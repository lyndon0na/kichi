//! `upload_queue.rs` 的调度状态测试；拆出来守住实现文件的行数提醒线。

use super::*;
use crate::cxxqt_object::transfers::{with_records, TransferDirection, TransferJob, UploadJob};
use std::path::PathBuf;

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn pending(batch: u64) -> Pending {
    let job = TransferJob {
        id: format!("upload:/tmp/test-{batch}.bin"),
        name: format!("test-{batch}.bin"),
        kind: EntryKind::File,
        size: 42,
        audit_flagged: false,
        sub_dir: Vec::new(),
        dir: PathBuf::new(),
        direction: TransferDirection::Upload,
        upload: Some(UploadJob {
            local_path: PathBuf::from(format!("/tmp/test-{batch}.bin")),
            parent_id: "parent".to_owned(),
            parent_name: "Test".to_owned(),
            ..UploadJob::default()
        }),
    };
    let batch_cancel = Arc::new(Cancel::default());
    Pending {
        batch,
        source_id: format!("upload:/tmp/test-{batch}.bin"),
        parent: None,
        files: vec![UploadFile {
            index: 0,
            job,
            cancel: Cancel::child(Arc::clone(&batch_cancel)),
        }]
        .into(),
        dirs: Vec::new().into(),
        upload_dirs: Vec::new().into(),
        batch_cancel,
        abandoned: false,
    }
}

fn pending_folder(batch: u64, file_count: usize) -> Pending {
    let mut pending = pending(batch);
    let parent_job = TransferJob {
        id: format!("upload-folder:/tmp/test-{batch}"),
        name: format!("test-{batch}"),
        kind: EntryKind::Folder,
        size: 0,
        audit_flagged: false,
        sub_dir: Vec::new(),
        dir: PathBuf::new(),
        direction: TransferDirection::Upload,
        upload: Some(UploadJob {
            local_path: PathBuf::from(format!("/tmp/test-{batch}")),
            parent_id: "parent".to_owned(),
            parent_name: "Test".to_owned(),
            ..UploadJob::default()
        }),
    };
    pending.parent = Some(parent_job);
    let batch_cancel = Arc::clone(&pending.batch_cancel);
    pending.files = (0..file_count)
        .map(|index| UploadFile {
            index,
            job: TransferJob {
                id: format!("upload:/tmp/test-{batch}/{index}.bin"),
                name: format!("{index}.bin"),
                kind: EntryKind::File,
                size: 42,
                audit_flagged: false,
                sub_dir: vec![format!("test-{batch}")],
                dir: PathBuf::new(),
                direction: TransferDirection::Upload,
                upload: Some(UploadJob {
                    local_path: PathBuf::from(format!("/tmp/test-{batch}/{index}.bin")),
                    parent_id: "parent".to_owned(),
                    parent_name: "Test".to_owned(),
                    ..UploadJob::default()
                }),
            },
            cancel: Cancel::child(Arc::clone(&batch_cancel)),
        })
        .collect::<Vec<_>>()
        .into();
    pending
}

fn clear_scheduler() {
    with_scheduler(|scheduler| {
        scheduler.queue.clear();
        scheduler.paused.clear();
        scheduler.active.clear();
        scheduler.blocked = None;
    });
}

#[test]
fn canonical_source_id_is_stable_and_direction_specific() {
    let id = source_id_for(Path::new("/tmp/a b.txt"));
    assert_eq!(id, "upload:/tmp/a b.txt");
}

#[test]
fn non_file_is_rejected_before_a_record_is_created() {
    let temp = std::env::temp_dir();
    assert!(matches!(
        prepare_job(&temp, "", "全部文件"),
        Err(ReserveError::Invalid(_))
    ));
}

#[test]
fn queued_upload_can_pause_and_resume_with_the_same_batch() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|err| err.into_inner());
    clear_scheduler();
    let batch = 9_000_001;
    with_scheduler(|scheduler| scheduler.queue.push_back(pending(batch)));

    assert_eq!(pause(batch), PauseOutcome::Queued);
    assert_eq!(counts(), (false, 0), "暂停项不占排队数");
    assert_eq!(pause(batch), PauseOutcome::NotCurrent, "不能重复暂停");

    assert!(resume(batch));
    assert_eq!(counts(), (false, 1), "继续后重新排队");
    assert!(!resume(batch), "已在队列中不能重复继续");

    assert_eq!(cancel(batch), Some(CancelOutcome::Queued));
    assert!(is_idle());
    clear_scheduler();
}

#[test]
fn running_upload_pauses_then_resumes_or_cancels() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|err| err.into_inner());
    clear_scheduler();
    let batch = 9_000_002;
    let pending = pending(batch);
    let cancel_flag = Arc::clone(&pending.batch_cancel);
    with_scheduler(|scheduler| scheduler.active = vec![pending]);

    assert_eq!(pause(batch), PauseOutcome::Pausing);
    assert!(cancel_flag.is_paused(), "运行中的上传先收到协作暂停信号");
    assert!(finish_paused(batch), "worker 收尾后保留原批次快照");
    assert!(resume(batch), "暂停后可继续");
    assert!(!cancel_flag.is_paused() && !cancel_flag.is_cancelled());

    assert_eq!(pause(batch), PauseOutcome::Queued);
    assert_eq!(cancel(batch), Some(CancelOutcome::Queued));
    assert!(cancel_flag.is_cancelled());
    assert!(!cancel_flag.is_paused(), "暂停后取消必须切回删除语义");
    clear_scheduler();
}

#[test]
fn cancel_while_pausing_does_not_leave_a_paused_snapshot() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|err| err.into_inner());
    clear_scheduler();
    let batch = 9_000_008;
    let pending = pending(batch);
    let cancel_flag = Arc::clone(&pending.batch_cancel);
    with_scheduler(|scheduler| scheduler.active = vec![pending]);

    assert_eq!(pause(batch), PauseOutcome::Pausing);
    assert_eq!(cancel(batch), Some(CancelOutcome::Running));
    assert!(cancel_flag.is_cancelled() && !cancel_flag.is_paused());
    assert!(
        !finish_paused(batch),
        "取消已覆盖暂停，不能留下无法继续的暂停快照"
    );
    with_scheduler(|scheduler| assert!(scheduler.paused.is_empty()));
    clear_scheduler();
}

#[test]
fn paused_upload_can_be_cancelled_without_a_worker() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|err| err.into_inner());
    clear_scheduler();
    let batch = 9_000_003;
    with_scheduler(|scheduler| scheduler.queue.push_back(pending(batch)));

    assert_eq!(pause(batch), PauseOutcome::Queued);
    assert_eq!(cancel(batch), Some(CancelOutcome::Queued));
    assert!(is_idle());
    with_scheduler(|scheduler| {
        assert!(scheduler.queue.is_empty());
        assert!(scheduler.paused.is_empty());
        assert!(scheduler.active.is_empty());
    });
    clear_scheduler();
}

#[test]
fn folder_retry_queues_all_selected_files_in_one_batch() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|err| err.into_inner());
    clear_scheduler();
    let batch = 9_000_004;
    let parent = pending_folder(batch, 2).parent.expect("文件夹父任务");
    let files = pending_folder(batch, 2)
        .files
        .iter()
        .map(|file| (file.index, file.job.clone()))
        .collect();

    retry_folder(
        batch,
        parent,
        files,
        vec![vec!["test-9-000-004".to_owned()]],
        Vec::new(),
    )
    .expect("文件夹批次可入队");
    assert_eq!(pending_file_count(batch), 2);
    clear_scheduler();
}

#[test]
fn batch_finished_and_running_helpers_only_consider_their_batch() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|err| err.into_inner());
    clear_scheduler();
    let batch = 9_000_005;
    let pending = pending_folder(batch, 2);
    let parent_job = pending.parent.clone().expect("父任务");
    let files = pending.files.to_vec();
    with_records(|records| {
        let mut parent =
            transfers::row_from(batch, 0, &parent_job.id, &parent_job, 1_790_000_000, 1);
        parent.state = transfers::TransferState::Running;
        records.push(parent);
        for file in files {
            let mut child = transfers::row_from(
                batch,
                file.index,
                &parent_job.id,
                &file.job,
                1_790_000_000,
                1,
            );
            child.child = true;
            child.state = if file.index == 0 {
                transfers::TransferState::Running
            } else {
                transfers::TransferState::Queued
            };
            records.push(child);
        }
    });

    assert_eq!(running_file_count(batch), 1);
    assert!(!batch_all_finished(batch));

    with_records(|records| {
        for record in records.iter_mut().filter(|record| record.batch == batch) {
            if record.child {
                record.state = transfers::TransferState::Done;
            }
        }
    });
    assert!(batch_all_finished(batch));

    with_records(|records| records.retain(|record| record.batch != batch));
    clear_scheduler();
}

#[test]
fn failure_cleanup_does_not_interrupt_another_parallel_batch() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|err| err.into_inner());
    clear_scheduler();
    let first = pending(9_000_006);
    let second = pending(9_000_007);
    let first_batch = first.batch;
    let second_batch = second.batch;
    with_records(|records| {
        records.clear();
        for batch in [&first, &second] {
            let file = &batch.files[0];
            let mut row = transfers::row_from(
                batch.batch,
                file.index,
                &batch.source_id,
                &file.job,
                1_790_000_000,
                1,
            );
            row.state = transfers::TransferState::Running;
            records.push(row);
        }
    });

    transfers::interrupt_active_batch(first_batch);

    with_records(|records| {
        assert_eq!(
            records
                .iter()
                .find(|record| record.batch == first_batch)
                .map(|record| record.state),
            Some(transfers::TransferState::Interrupted)
        );
        assert_eq!(
            records
                .iter()
                .find(|record| record.batch == second_batch)
                .map(|record| record.state),
            Some(transfers::TransferState::Running),
            "另一个并发批次的结果不能被停队收尾吞掉"
        );
        records.clear();
    });
}

#[test]
fn single_file_batches_fill_parallel_slots_and_folder_batches_are_exclusive() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|err| err.into_inner());
    clear_scheduler();
    std::env::remove_var(crate::session::limits::UPLOAD_PARALLEL_ENV);
    crate::session::limits::configure_uploads(2, 0);

    with_scheduler(|scheduler| {
        scheduler.queue.push_back(pending(9_100_001));
        scheduler.queue.push_back(pending(9_100_002));
        scheduler.queue.push_back(pending(9_100_003));
    });
    assert!(next_start().is_some());
    assert!(next_start().is_some());
    assert!(next_start().is_none(), "两个单文件槽已满");
    assert_eq!(
        with_scheduler(|scheduler| scheduler.active.len()),
        2,
        "单文件批次应并行占用两个槽"
    );

    assert!(release(9_100_001));
    assert!(next_start().is_some(), "释放一个槽后按队列顺序补位");
    assert_eq!(
        with_scheduler(|scheduler| scheduler.active[1].batch),
        9_100_003
    );

    clear_scheduler();
    with_scheduler(|scheduler| {
        scheduler.queue.push_back(pending(9_100_004));
        scheduler.queue.push_back(pending_folder(9_100_005, 2));
        scheduler.queue.push_back(pending(9_100_006));
    });
    assert!(next_start().is_some(), "先启动前面的单文件");
    assert!(next_start().is_none(), "队首文件夹要等单文件槽全部空出来");
    assert!(release(9_100_004));
    assert!(
        next_start()
            .as_ref()
            .is_some_and(|start| start.parent.is_some()),
        "单文件槽清空后应启动文件夹批次"
    );
    assert!(
        next_start().is_none(),
        "文件夹运行期间不能再启动后面的单文件"
    );
    assert!(release(9_100_005));
    assert!(next_start().is_some(), "文件夹结束后继续启动单文件");

    crate::session::limits::configure_uploads(crate::session::limits::DEFAULT_UPLOAD_PARALLEL, 0);
    clear_scheduler();
}

#[test]
fn failure_halt_blocks_new_starts_until_new_work_is_accepted() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|err| err.into_inner());
    clear_scheduler();
    let batch = 9_100_007;
    let pending = pending(batch);
    let cancel = Arc::clone(&pending.batch_cancel);
    with_scheduler(|scheduler| scheduler.active.push(pending));

    halt_for_failure("429");
    assert!(cancel.is_halted());
    assert!(!cancel.is_abandoned(), "限流停队不是用户取消");
    assert!(next_start().is_none(), "停队后不再启动新批次");

    with_scheduler(accept_new_work);
    let _ = release(batch);
    clear_scheduler();
}
