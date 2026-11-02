//! `upload_queue.rs` 的调度状态测试；拆出来守住实现文件的行数提醒线。

use super::*;
use crate::cxxqt_object::transfers::{TransferDirection, TransferJob, UploadJob};
use std::path::PathBuf;

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn pending(batch: u64) -> Pending {
    Pending {
        batch,
        source_id: format!("upload:/tmp/test-{batch}.bin"),
        job: TransferJob {
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
            }),
        },
        cancel: Arc::new(Cancel::default()),
        abandoned: false,
    }
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
    let job = pending(batch);
    let cancel_flag = Arc::clone(&job.cancel);
    with_scheduler(|scheduler| scheduler.active = Some(job));

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
