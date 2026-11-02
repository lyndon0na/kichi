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
        }],
        dirs: Vec::new(),
        upload_dirs: Vec::new(),
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
        .collect();
    pending
}

fn clear_scheduler() {
    with_scheduler(|scheduler| {
        scheduler.queue.clear();
        scheduler.paused.clear();
        scheduler.active = None;
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
fn running_upload_cannot_pause_but_can_cancel() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|err| err.into_inner());
    clear_scheduler();
    let batch = 9_000_002;
    let pending = pending(batch);
    let cancel_flag = Arc::clone(&pending.batch_cancel);
    with_scheduler(|scheduler| scheduler.active = Some(pending));

    assert_eq!(pause(batch), PauseOutcome::Running);
    assert_eq!(cancel(batch), Some(CancelOutcome::Running));
    assert!(cancel_flag.is_cancelled());
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
        assert!(scheduler.active.is_none());
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
        .into_iter()
        .map(|file| (file.index, file.job))
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
    let mut pending = pending_folder(batch, 2);
    let parent_job = pending.parent.clone().expect("父任务");
    let files = std::mem::take(&mut pending.files);
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
