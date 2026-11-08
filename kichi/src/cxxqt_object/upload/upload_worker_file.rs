//! 上传文件循环：单文件与文件夹内文件的并发上传、阶段进度。

use std::sync::Arc;

use kichi_core::{DriveClient, UploadResult};

use super::transfers::{self, TransferState, UploadDir, UploadProgressState};
use super::upload_queue;
use super::upload_worker::{same_epoch, BatchOutcome, FileWorkResult, UploadFile, UploadStart};
use super::upload_worker_notice::{
    fatal_failure, post_live, post_sync, record_upload_end, ProgressThrottle, UploadEnd,
};
use crate::session::limits;
use crate::session::{Failure, Session};

pub(crate) fn upload_folder_file(
    session: &Arc<Session>,
    client: &Arc<DriveClient>,
    start: &UploadStart,
    target_parent: &str,
    upload_dirs: &[UploadDir],
    file: &UploadFile,
    my_epoch: u64,
) -> FileWorkResult {
    if !same_epoch(my_epoch) || start.batch_cancel.is_abandoned() {
        return FileWorkResult::Interrupted;
    }
    if start.batch_cancel.is_halted() || file.cancel.is_halted() {
        return FileWorkResult::Interrupted;
    }
    if start.batch_cancel.is_paused() || file.cancel.is_paused() {
        return FileWorkResult::Paused;
    }
    if start.batch_cancel.is_cancelled() {
        return FileWorkResult::BatchCancelled;
    }
    if file.cancel.is_local_cancelled() {
        if same_epoch(my_epoch) {
            transfers::cancel_file(start.batch, file.index);
            record_upload_end(&UploadEnd::Cancelled);
            post_sync();
        }
        return FileWorkResult::LocalCancelled;
    }
    if transfers::upload_child_state(start.batch, file.index) != Some(TransferState::Queued) {
        return FileWorkResult::Skipped;
    }
    let parent_id = if file.job.sub_dir.is_empty() {
        Some(target_parent.to_owned())
    } else {
        upload_dirs
            .iter()
            .find(|dir| dir.path == file.job.sub_dir)
            .map(|dir| dir.id.clone())
    };
    let Some(parent_id) = parent_id else {
        let reason = "云端目录没有准备成功".to_owned();
        if same_epoch(my_epoch) {
            transfers::finish_err(start.batch, file.index, &reason);
            record_upload_end(&UploadEnd::Failed(reason));
        }
        return FileWorkResult::Failed {
            failure: Failure::Other("云端目录没有准备成功".to_owned()),
            stop_queue: false,
        };
    };
    if same_epoch(my_epoch) {
        transfers::set_upload_file_parent(start.batch, file.index, &parent_id);
        transfers::begin(start.batch, file.index);
        // 开始上传是结构变化；只发进度刷新不会把 QML 里的「排队中」改成上传阶段。
        post_sync();
    }
    match upload_one(session, client, start.batch, file, &parent_id) {
        Ok(result) => {
            if same_epoch(my_epoch) {
                transfers::finish_upload_ok(start.batch, file.index, &result);
                record_upload_end(&UploadEnd::Done);
                post_sync();
            }
            FileWorkResult::Success
        }
        Err(_) if !same_epoch(my_epoch) || start.batch_cancel.is_abandoned() => {
            FileWorkResult::Interrupted
        }
        Err(_) if start.batch_cancel.is_halted() || file.cancel.is_halted() => {
            FileWorkResult::Interrupted
        }
        Err(_) if start.batch_cancel.is_paused() || file.cancel.is_paused() => {
            FileWorkResult::Paused
        }
        Err(_) if start.batch_cancel.is_cancelled() => FileWorkResult::BatchCancelled,
        Err(_) if file.cancel.is_local_cancelled() => {
            if same_epoch(my_epoch) {
                transfers::cancel_file(start.batch, file.index);
                record_upload_end(&UploadEnd::Cancelled);
            }
            FileWorkResult::LocalCancelled
        }
        Err(error) => {
            let stop_queue = error.stop_queue || fatal_failure(&error.failure);
            let reason = error.failure.to_string();
            if same_epoch(my_epoch) {
                transfers::finish_err(start.batch, file.index, &reason);
                record_upload_end(&UploadEnd::Failed(reason.clone()));
                post_sync();
            }
            if stop_queue {
                upload_queue::halt_for_failure(&reason);
            }
            FileWorkResult::Failed {
                failure: error.failure,
                stop_queue,
            }
        }
    }
}

pub(crate) fn run_single_file(
    session: &Arc<Session>,
    start: &UploadStart,
    my_epoch: u64,
    outcome: &mut BatchOutcome,
) {
    let Some(file) = start.files.first() else {
        outcome.fatal = Some(Failure::Other("上传任务缺少文件".to_owned()));
        return;
    };
    if !same_epoch(my_epoch) || start.batch_cancel.is_abandoned() {
        outcome.interrupted = true;
        return;
    }
    if start.batch_cancel.is_halted() || file.cancel.is_halted() {
        outcome.interrupted = true;
        return;
    }
    if start.batch_cancel.is_paused() || file.cancel.is_paused() {
        outcome.paused = true;
        return;
    }
    if start.batch_cancel.is_cancelled() {
        outcome.batch_cancelled = true;
        if same_epoch(my_epoch) {
            transfers::cancel_batch(start.batch, "");
        }
        return;
    }
    let client = match session.drive() {
        Ok(client) => client,
        Err(err) => {
            let reason = err.to_string();
            let stop_queue = fatal_failure(&err);
            if same_epoch(my_epoch) {
                transfers::finish_err(start.batch, file.index, &reason);
                record_upload_end(&UploadEnd::Failed(reason.clone()));
                outcome.file_end_recorded = true;
            }
            if stop_queue {
                if same_epoch(my_epoch) {
                    transfers::fail_batch(start.batch, &reason);
                }
                upload_queue::halt_for_failure(&reason);
                outcome.fatal = Some(err);
            }
            return;
        }
    };
    let Some(upload) = file.job.upload.as_ref() else {
        let reason = "上传任务缺少本地文件信息".to_owned();
        if same_epoch(my_epoch) {
            transfers::finish_err(start.batch, file.index, &reason);
            record_upload_end(&UploadEnd::Failed(reason));
            outcome.file_end_recorded = true;
        }
        outcome.fatal = Some(Failure::Other("上传任务缺少本地文件信息".to_owned()));
        return;
    };
    if same_epoch(my_epoch) {
        transfers::set_upload_file_parent(start.batch, file.index, &upload.parent_id);
        transfers::begin(start.batch, file.index);
        // 同上：phase 从排队变上传中必须走结构同步，不能让并发批次停在旧快照。
        post_sync();
    }
    match upload_one(session, &client, start.batch, file, &upload.parent_id) {
        Ok(result) => {
            if same_epoch(my_epoch) {
                transfers::finish_upload_ok(start.batch, file.index, &result);
                record_upload_end(&UploadEnd::Done);
                outcome.file_end_recorded = true;
                outcome.success_parent = Some(upload.parent_id.clone());
                post_sync();
            }
        }
        Err(_) if !same_epoch(my_epoch) || start.batch_cancel.is_abandoned() => {
            outcome.interrupted = true;
        }
        Err(_) if start.batch_cancel.is_halted() || file.cancel.is_halted() => {
            outcome.interrupted = true;
        }
        Err(_) if start.batch_cancel.is_paused() || file.cancel.is_paused() => {
            outcome.paused = true;
        }
        Err(_) if start.batch_cancel.is_cancelled() => {
            outcome.batch_cancelled = true;
            if same_epoch(my_epoch) {
                transfers::cancel_batch(start.batch, "");
                record_upload_end(&UploadEnd::Cancelled);
                outcome.file_end_recorded = true;
            }
        }
        Err(_) if file.cancel.is_local_cancelled() => {
            if same_epoch(my_epoch) {
                transfers::cancel_file(start.batch, file.index);
                record_upload_end(&UploadEnd::Cancelled);
                outcome.file_end_recorded = true;
                post_sync();
            }
        }
        Err(error) => {
            let stop_queue = error.stop_queue || fatal_failure(&error.failure);
            let reason = error.failure.to_string();
            if same_epoch(my_epoch) {
                transfers::finish_err(start.batch, file.index, &reason);
                record_upload_end(&UploadEnd::Failed(reason.clone()));
                outcome.file_end_recorded = true;
                post_sync();
            }
            if stop_queue {
                if same_epoch(my_epoch) {
                    transfers::fail_batch(start.batch, &reason);
                }
                upload_queue::halt_for_failure(&reason);
                outcome.fatal = Some(error.failure);
            }
        }
    }
}

pub(crate) fn upload_one(
    session: &Arc<Session>,
    client: &Arc<DriveClient>,
    batch: u64,
    file: &UploadFile,
    parent_id: &str,
) -> Result<UploadResult, crate::session::upload::SessionUploadError> {
    let upload =
        file.job
            .upload
            .as_ref()
            .ok_or_else(|| crate::session::upload::SessionUploadError {
                failure: Failure::Other("上传任务缺少本地文件信息".to_owned()),
                keep_resume: false,
                stop_queue: false,
            })?;
    let mut throttle = ProgressThrottle::default();
    session.upload_file(
        Arc::clone(client),
        &upload.local_path,
        parent_id,
        Arc::clone(&file.cancel),
        Some(limits::upload_limiter()),
        |progress| {
            if throttle.should_send(progress) {
                transfers::upload_progress(
                    batch,
                    file.index,
                    UploadProgressState {
                        stage: progress.stage,
                        bytes: progress.bytes,
                        completed_bytes: progress.completed_bytes,
                        total: progress.total,
                    },
                );
                // 阶段名（准备 / 校验 / 上传 / 收尾）在主模型里，只有整表同步
                // 才会更新；百分比和速度仍走轻量 live，避免整页重灌。
                if throttle.stage_changed() {
                    post_sync();
                } else {
                    post_live();
                }
            }
        },
    )
}
