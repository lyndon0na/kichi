//! 上传文件夹：目录链规划、目录失败隔离与整批收尾。

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use kichi_core::DriveClient;

use super::transfers::{self, UploadDir};
use super::upload_queue;
use super::upload_worker::{
    same_epoch, BatchOutcome, FilePhaseOutcome, FileWorkResult, UploadStart,
};
use super::upload_worker_file::upload_folder_file;
use super::upload_worker_notice::{record_upload_end, UploadEnd};
use crate::session::{limits, Failure, Session};

pub(crate) fn run_folder_files(
    session: &Arc<Session>,
    client: &Arc<DriveClient>,
    start: &UploadStart,
    target_parent: &str,
    upload_dirs: &[UploadDir],
    my_epoch: u64,
) -> FilePhaseOutcome {
    let state = Mutex::new(FilePhaseOutcome::default());
    let stop = AtomicBool::new(false);
    let panicked = AtomicBool::new(false);
    let cursor = AtomicUsize::new(0);
    let workers = limits::upload_parallel().min(start.files.len().max(1));
    let work = || loop {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let index = cursor.fetch_add(1, Ordering::SeqCst);
        let Some(file) = start.files.get(index) else {
            break;
        };
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let result = upload_folder_file(
            session,
            client,
            start,
            target_parent,
            upload_dirs,
            file,
            my_epoch,
        );
        let mut outcome = state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match result {
            FileWorkResult::Success => {
                outcome.success = true;
                outcome.file_end_recorded = true;
            }
            FileWorkResult::Paused => {
                outcome.paused = true;
                stop.store(true, Ordering::SeqCst);
            }
            FileWorkResult::Interrupted => outcome.interrupted = true,
            FileWorkResult::BatchCancelled => outcome.batch_cancelled = true,
            FileWorkResult::LocalCancelled => outcome.file_end_recorded = true,
            FileWorkResult::Failed {
                failure,
                stop_queue,
            } => {
                outcome.file_end_recorded = true;
                if stop_queue {
                    stop.store(true, Ordering::SeqCst);
                    if outcome.fatal.is_none() {
                        outcome.fatal = Some(failure);
                    }
                }
            }
            FileWorkResult::Skipped => {}
        }
    };
    let work = &work;

    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for number in 1..workers {
            let panic_flag = &panicked;
            let stop_flag = &stop;
            let spawned = std::thread::Builder::new()
                .name(format!("kichi-upload-{number}"))
                .spawn_scoped(scope, move || {
                    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)).is_err() {
                        panic_flag.store(true, Ordering::SeqCst);
                        stop_flag.store(true, Ordering::SeqCst);
                    }
                });
            match spawned {
                Ok(handle) => handles.push(handle),
                Err(err) => eprintln!("[kichi] 无法创建上传线程（{number}）：{err}"),
            }
        }
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)).is_err() {
            panicked.store(true, Ordering::SeqCst);
            stop.store(true, Ordering::SeqCst);
        }
        for handle in handles {
            if handle.join().is_err() {
                panicked.store(true, Ordering::SeqCst);
                stop.store(true, Ordering::SeqCst);
            }
        }
    });

    let mut outcome = state
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if panicked.load(Ordering::SeqCst) {
        let reason = "上传线程异常中断".to_owned();
        if same_epoch(my_epoch) {
            transfers::fail_batch(start.batch, &reason);
            record_upload_end(&UploadEnd::Failed(reason.clone()));
        }
        outcome.file_end_recorded = true;
        outcome.interrupted = true;
        outcome.fatal = Some(Failure::Other(reason));
    }
    outcome
}

pub(crate) fn finish_folder_outcome(
    start: &UploadStart,
    mut outcome: BatchOutcome,
) -> BatchOutcome {
    if same_epoch(upload_queue::current_epoch()) && start.parent.is_some() {
        if outcome.paused {
            // 父行与仍在途的子行由 `post_finished` 统一落成「已暂停」。
        } else if outcome.interrupted {
            transfers::interrupt_active_batch(start.batch);
            transfers::finish_upload_folder(start.batch);
        } else {
            transfers::finish_upload_folder(start.batch);
        }
    }
    if !outcome.paused && !outcome.file_end_recorded && same_epoch(upload_queue::current_epoch()) {
        if let Some(failure) = outcome.fatal.as_ref() {
            record_upload_end(&UploadEnd::Failed(failure.to_string()));
        } else if outcome.batch_cancelled {
            record_upload_end(&UploadEnd::Cancelled);
        } else if outcome.interrupted {
            record_upload_end(&UploadEnd::Interrupted);
        } else if outcome.dir_failures > 0 {
            record_upload_end(&UploadEnd::Failed(
                outcome
                    .first_dir_error
                    .clone()
                    .unwrap_or_else(|| "云端目录创建失败".to_owned()),
            ));
        } else if start.parent.is_some() {
            record_upload_end(&UploadEnd::Done);
            outcome.success_parent = start
                .parent
                .as_ref()
                .and_then(|parent| parent.upload.as_ref())
                .map(|upload| upload.parent_id.clone());
        }
    }
    outcome
}

pub(crate) fn fail_dir(
    start: &UploadStart,
    path: &[String],
    reason: &str,
    outcome: &mut BatchOutcome,
    failed_paths: &mut Vec<Vec<String>>,
) {
    if same_epoch(upload_queue::current_epoch()) {
        transfers::fail_upload_subtree(start.batch, path, reason);
    }
    if !failed_paths.iter().any(|failed| failed.as_slice() == path) {
        failed_paths.push(path.to_vec());
    }
    outcome.dir_failures += 1;
    if outcome.first_dir_error.is_none() {
        outcome.first_dir_error = Some(reason.to_owned());
    }
}

pub(crate) fn all_dirs(start: &UploadStart) -> Vec<Vec<String>> {
    let mut dirs = start.dirs.to_vec();
    for file in start.files.iter() {
        for depth in 1..=file.job.sub_dir.len() {
            dirs.push(file.job.sub_dir[..depth].to_vec());
        }
    }
    dirs.sort_by(|a, b| a.len().cmp(&b.len()).then_with(|| a.cmp(b)));
    dirs.dedup();
    dirs
}

pub(crate) fn parent_for_path(
    path: &[String],
    target_parent: &str,
    upload_dirs: &[UploadDir],
) -> Option<String> {
    if path.len() == 1 {
        return Some(target_parent.to_owned());
    }
    let parent_path = &path[..path.len() - 1];
    upload_dirs
        .iter()
        .find(|dir| dir.path == parent_path)
        .map(|dir| dir.id.clone())
}

pub(crate) fn path_starts_with(path: &[String], prefix: &[String]) -> bool {
    path.len() >= prefix.len() && path[..prefix.len()] == *prefix
}
