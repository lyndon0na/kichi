//! 上传线程入口：批次编排，目录链之后转入文件上传。
//!
//! 后台线程绝不碰 Qt 对象；结构变化和实时进度分别经 `enqueue_ui` 回主线程。
//! 目录链在 [`super::upload_worker_folder`]，文件循环在
//! [`super::upload_worker_file`]，通知与进度回投在 [`super::upload_worker_notice`]。

use std::sync::Arc;

use kichi_core::FolderCreateCache;

use super::transfers::{self, TransferJob, UploadDir};
use super::upload_queue;
use crate::session::download::Cancel;
use crate::session::{Failure, Session};

#[derive(Clone)]
pub(crate) struct UploadFile {
    pub(crate) index: usize,
    pub(crate) job: TransferJob,
    pub(crate) cancel: Arc<Cancel>,
}

#[derive(Clone)]
pub(crate) struct UploadStart {
    pub(crate) batch: u64,
    pub(crate) batch_cancel: Arc<Cancel>,
    pub(crate) parent: Option<TransferJob>,
    pub(crate) files: Arc<[UploadFile]>,
    pub(crate) dirs: Arc<[Vec<String>]>,
    pub(crate) upload_dirs: Arc<[UploadDir]>,
}

pub(crate) fn spawn_upload(session: &Arc<Session>, start: &UploadStart) -> std::io::Result<()> {
    let session = Arc::clone(session);
    let start = start.clone();
    let my_epoch = upload_queue::current_epoch();
    std::thread::Builder::new()
        .name("kichi-upload".into())
        .spawn(move || run(session, start, my_epoch))
        .map(|_| ())
}

fn run(session: Arc<Session>, start: UploadStart, my_epoch: u64) {
    let batch = start.batch;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_batch(&session, &start, my_epoch)
    }));
    let outcome = match result {
        Ok(outcome) => outcome,
        Err(_) => {
            let reason = "上传线程异常中断".to_owned();
            if same_epoch(my_epoch) {
                if start.parent.is_some() {
                    transfers::fail_batch(batch, &reason);
                } else if let Some(file) = start.files.first() {
                    transfers::finish_err(batch, file.index, &reason);
                }
                record_upload_end(&UploadEnd::Failed(reason.clone()));
            }
            BatchOutcome {
                fatal: Some(Failure::Other(reason)),
                ..BatchOutcome::default()
            }
        }
    };
    if outcome.interrupted && same_epoch(my_epoch) {
        transfers::interrupt_active_batch(batch);
        if start.parent.is_some() {
            transfers::finish_upload_folder(batch);
        }
    }
    post_finished(batch, outcome, my_epoch);
}

#[derive(Default)]
pub(crate) struct BatchOutcome {
    pub(crate) fatal: Option<Failure>,
    pub(crate) success_parent: Option<String>,
    pub(crate) paused: bool,
    pub(crate) interrupted: bool,
    pub(crate) batch_cancelled: bool,
    pub(crate) dir_failures: usize,
    pub(crate) first_dir_error: Option<String>,
    pub(crate) file_end_recorded: bool,
}

#[derive(Default)]
pub(crate) struct FilePhaseOutcome {
    pub(crate) fatal: Option<Failure>,
    pub(crate) success: bool,
    pub(crate) paused: bool,
    pub(crate) interrupted: bool,
    pub(crate) batch_cancelled: bool,
    pub(crate) file_end_recorded: bool,
}

pub(crate) enum FileWorkResult {
    Success,
    Paused,
    Interrupted,
    BatchCancelled,
    LocalCancelled,
    Failed { failure: Failure, stop_queue: bool },
    Skipped,
}

fn run_batch(session: &Arc<Session>, start: &UploadStart, my_epoch: u64) -> BatchOutcome {
    let mut outcome = BatchOutcome::default();
    let Some(parent) = start.parent.as_ref() else {
        run_single_file(session, start, my_epoch, &mut outcome);
        return outcome;
    };
    let target_parent = parent
        .upload
        .as_ref()
        .map(|upload| upload.parent_id.clone())
        .unwrap_or_default();

    let client = match session.drive() {
        Ok(client) => client,
        Err(err) => {
            let reason = err.to_string();
            if same_epoch(my_epoch) {
                transfers::fail_batch(start.batch, &reason);
                record_upload_end(&UploadEnd::Failed(reason));
            }
            outcome.fatal = Some(err);
            return outcome;
        }
    };

    if !same_epoch(my_epoch) || start.batch_cancel.is_abandoned() {
        outcome.interrupted = true;
        return outcome;
    }
    if start.batch_cancel.is_halted() {
        outcome.interrupted = true;
        return outcome;
    }
    if start.batch_cancel.is_paused() {
        outcome.paused = true;
        return outcome;
    }
    if start.batch_cancel.is_cancelled() {
        outcome.batch_cancelled = true;
        if same_epoch(my_epoch) {
            transfers::cancel_batch(start.batch, "");
        }
        return outcome;
    }

    if same_epoch(my_epoch) {
        transfers::mark_upload_folder_running(start.batch);
    }
    let dirs = all_dirs(start);
    let mut upload_dirs = start.upload_dirs.to_vec();
    if let Some(upload) = parent.upload.as_ref() {
        if !upload.batch_root_id.is_empty() {
            if let Some(root_path) = dirs.iter().find(|path| path.len() == 1) {
                if !upload_dirs.iter().any(|dir| dir.path == *root_path) {
                    upload_dirs.push(UploadDir {
                        path: root_path.clone(),
                        id: upload.batch_root_id.clone(),
                    });
                }
            }
        }
    }
    let mut failed_paths: Vec<Vec<String>> = Vec::new();
    if same_epoch(my_epoch) {
        transfers::set_upload_folder_detail(start.batch, "正在创建云端目录…");
        // 父行转「上传中」和说明变化都是结构变化；只发进度刷新不会改 QML 的 phase。
        post_sync();
    }

    let mut folder_cache = FolderCreateCache::new();
    for (position, path) in dirs.iter().enumerate() {
        if !same_epoch(my_epoch) || start.batch_cancel.is_abandoned() {
            outcome.interrupted = true;
            return finish_folder_outcome(start, outcome);
        }
        if start.batch_cancel.is_halted() {
            outcome.interrupted = true;
            return finish_folder_outcome(start, outcome);
        }
        if start.batch_cancel.is_paused() {
            outcome.paused = true;
            return finish_folder_outcome(start, outcome);
        }
        if start.batch_cancel.is_cancelled() {
            outcome.batch_cancelled = true;
            if same_epoch(my_epoch) {
                transfers::cancel_batch(start.batch, "");
            }
            return finish_folder_outcome(start, outcome);
        }
        if upload_dirs.iter().any(|dir| dir.path == *path) {
            continue;
        }
        if failed_paths
            .iter()
            .any(|failed| path_starts_with(path, failed))
        {
            continue;
        }

        let Some(parent_id) = parent_for_path(path, &target_parent, &upload_dirs) else {
            let reason = "上级云端目录没有创建成功".to_owned();
            fail_dir(start, path, &reason, &mut outcome, &mut failed_paths);
            continue;
        };
        let name = path.last().cloned().unwrap_or_default();
        if name.trim().is_empty() {
            let reason = "云端目录名称为空".to_owned();
            fail_dir(start, path, &reason, &mut outcome, &mut failed_paths);
            continue;
        }
        if same_epoch(my_epoch) {
            transfers::set_upload_folder_detail(
                start.batch,
                &format!(
                    "正在创建云端目录 {}/{}：{}",
                    position + 1,
                    dirs.len(),
                    path.join("/")
                ),
            );
            post_sync();
        }
        match session.create_folder_entry(
            Arc::clone(&client),
            Some(&parent_id),
            &name,
            &mut folder_cache,
        ) {
            Ok(id) => {
                upload_dirs.push(UploadDir {
                    path: path.clone(),
                    id,
                });
                if path.len() == 1 {
                    // 根目录已经建出来，即使随后所有文件都失败，目标目录的
                    // 云端列表也需要刷新一次来显示这个新文件夹。
                    outcome.success_parent = Some(target_parent.clone());
                }
                if same_epoch(my_epoch) {
                    transfers::set_upload_dirs(start.batch, &upload_dirs);
                    transfers::clear_upload_dir_failure(start.batch, path);
                    post_sync();
                }
            }
            Err(err) => {
                let reason = err.to_string();
                fail_dir(start, path, &reason, &mut outcome, &mut failed_paths);
                if fatal_failure(&err) {
                    if same_epoch(my_epoch) {
                        transfers::fail_batch(start.batch, &reason);
                    }
                    upload_queue::halt_for_failure(&reason);
                    outcome.fatal = Some(err);
                    break;
                }
            }
        }
    }

    if outcome.fatal.is_some() || outcome.paused || outcome.batch_cancelled || outcome.interrupted {
        if let Some(failure) = outcome.fatal.as_ref() {
            if same_epoch(my_epoch) {
                transfers::fail_batch(start.batch, &failure.to_string());
            }
        }
        return finish_folder_outcome(start, outcome);
    }

    if same_epoch(my_epoch) {
        // 目录链已处理完；文件循环开始时父行说明应回到“第 x/y 个”。
        // 若继续保留“正在准备上传文件…”，`upload_folder_detail()` 会一直
        // 走临时说明分支，整个上传阶段都卡在这句话上。
        transfers::set_upload_folder_detail(start.batch, "");
        post_sync();
    }
    let phase = run_folder_files(
        session,
        &client,
        start,
        &target_parent,
        &upload_dirs,
        my_epoch,
    );
    outcome.fatal = phase.fatal;
    outcome.paused |= phase.paused;
    outcome.interrupted |= phase.interrupted;
    outcome.batch_cancelled |= phase.batch_cancelled;
    outcome.file_end_recorded |= phase.file_end_recorded;
    if phase.success {
        outcome.success_parent = Some(target_parent.clone());
    }
    if outcome.batch_cancelled && same_epoch(my_epoch) {
        transfers::cancel_batch(start.batch, "");
    }

    if let Some(failure) = outcome.fatal.as_ref() {
        if same_epoch(my_epoch) {
            transfers::fail_batch(start.batch, &failure.to_string());
        }
    }
    finish_folder_outcome(start, outcome)
}

pub(crate) fn same_epoch(my_epoch: u64) -> bool {
    my_epoch == upload_queue::current_epoch()
}

// 通知与进度的实现在 [`super::upload_worker_notice`]，这里重导出给子模块与测试。
use super::upload_worker_file::run_single_file;
use super::upload_worker_folder::{
    all_dirs, fail_dir, finish_folder_outcome, parent_for_path, path_starts_with, run_folder_files,
};
pub(crate) use super::upload_worker_notice::{
    clear_notice_group, fatal_failure, post_finished, post_sync, record_upload_end, UploadEnd,
};

#[cfg(test)]
#[path = "upload_worker_mock_tests.rs"]
mod mock_tests;
