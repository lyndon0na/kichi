//! 上传线程：取得会话 client、建上传文件夹的目录链、并发上传文件并回投进度。
//!
//! 后台线程绝不碰 Qt 对象；结构变化和实时进度分别经 `enqueue_ui` 回主线程。
//! 文件夹批次先由一个线程建目录链，再按上传并发数启动文件 worker。

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use kichi_core::{DriveClient, FolderCreateCache, UploadProgress, UploadResult, UploadStage};

use super::transfers::{self, TransferJob, TransferState, UploadDir, UploadProgressState};
use super::transfers_sync;
use super::upload_queue;
use crate::session::download::Cancel;
use crate::session::limits;
use crate::session::{Failure, Session};

const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Default)]
struct UploadNoticeGroup {
    done: u32,
    failed: u32,
    cancelled: u32,
    interrupted: u32,
    failure: Option<String>,
}

fn notice_group() -> &'static Mutex<UploadNoticeGroup> {
    static GROUP: OnceLock<Mutex<UploadNoticeGroup>> = OnceLock::new();
    GROUP.get_or_init(|| Mutex::new(UploadNoticeGroup::default()))
}

fn record_upload_end(end: &UploadEnd) {
    let mut group = notice_group()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    match end {
        UploadEnd::Done => group.done = group.done.saturating_add(1),
        UploadEnd::Failed(reason) => {
            group.failed = group.failed.saturating_add(1);
            group.failure = Some(reason.clone());
        }
        UploadEnd::Cancelled => group.cancelled = group.cancelled.saturating_add(1),
        UploadEnd::Interrupted => group.interrupted = group.interrupted.saturating_add(1),
    }
}

fn take_upload_notice() -> Option<super::notifications::Notice> {
    let group = {
        let mut guard = notice_group()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        std::mem::take(&mut *guard)
    };
    super::notifications::upload_finished(super::notifications::UploadSummary {
        done: group.done,
        failed: group.failed,
        cancelled: group.cancelled,
        interrupted: group.interrupted,
        failure: group.failure.as_deref(),
    })
}

pub(crate) fn clear_notice_group() {
    *notice_group()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = UploadNoticeGroup::default();
}

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
struct BatchOutcome {
    fatal: Option<Failure>,
    success_parent: Option<String>,
    paused: bool,
    interrupted: bool,
    batch_cancelled: bool,
    dir_failures: usize,
    first_dir_error: Option<String>,
    file_end_recorded: bool,
}

#[derive(Default)]
struct FilePhaseOutcome {
    fatal: Option<Failure>,
    success: bool,
    paused: bool,
    interrupted: bool,
    batch_cancelled: bool,
    file_end_recorded: bool,
}

enum FileWorkResult {
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

fn run_folder_files(
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

fn upload_folder_file(
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

fn run_single_file(
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

fn finish_folder_outcome(start: &UploadStart, mut outcome: BatchOutcome) -> BatchOutcome {
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

fn fail_dir(
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

fn all_dirs(start: &UploadStart) -> Vec<Vec<String>> {
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

fn parent_for_path(
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

fn path_starts_with(path: &[String], prefix: &[String]) -> bool {
    path.len() >= prefix.len() && path[..prefix.len()] == *prefix
}

fn same_epoch(my_epoch: u64) -> bool {
    my_epoch == upload_queue::current_epoch()
}

fn upload_one(
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

/// 首帧、阶段切换、整百分比变化或 250ms 到点时回投。
#[derive(Default)]
struct ProgressThrottle {
    last_sent: Option<Instant>,
    stage: Option<UploadStage>,
    percent: i32,
    stage_changed: bool,
}

impl ProgressThrottle {
    fn should_send(&mut self, progress: UploadProgress) -> bool {
        let percent = if progress.total > 0 {
            ((progress.bytes as f64 / progress.total as f64) * 100.0) as i32
        } else {
            -1
        };
        let now = Instant::now();
        let first = self.last_sent.is_none();
        let due = self
            .last_sent
            .is_some_and(|last| now.saturating_duration_since(last) >= PROGRESS_INTERVAL);
        let stage_changed = self.stage != Some(progress.stage);
        let changed = stage_changed || self.percent != percent;
        if first || due || changed {
            self.last_sent = Some(now);
            self.stage = Some(progress.stage);
            self.percent = percent;
            self.stage_changed = stage_changed;
            true
        } else {
            self.stage_changed = false;
            false
        }
    }

    fn stage_changed(&self) -> bool {
        self.stage_changed
    }
}

#[derive(Debug)]
enum UploadEnd {
    Done,
    Failed(String),
    Cancelled,
    Interrupted,
}

fn post_finished(batch: u64, outcome: BatchOutcome, my_epoch: u64) {
    super::actions::enqueue_ui(
        move |mut qichi: std::pin::Pin<&mut super::qobject::Kichi>| {
            let current = same_epoch(my_epoch);
            let paused = outcome.paused;
            let kept_paused = current && paused && upload_queue::finish_paused(batch);
            if !kept_paused {
                let _ = upload_queue::release(batch);
            }
            if current {
                if paused {
                    transfers::mark_paused(batch);
                }
                if let Some(parent_id) = outcome.success_parent.as_ref() {
                    qichi.as_mut().on_upload_succeeded(parent_id);
                }
                if let Some(failure) = outcome.fatal {
                    let resets = resets_upload_session(&failure);
                    if resets {
                        qichi.as_mut().reset_uploads();
                    }
                    qichi.as_mut().on_failure(failure);
                }
                if !paused && upload_queue::is_idle() {
                    if let Some(notice) = take_upload_notice() {
                        if qichi.notify_enabled {
                            super::notifications::show_notice(&notice);
                        }
                    }
                }
                qichi.as_mut().sync_transfers();
            }
            qichi.as_mut().advance_uploads();
        },
    );
}

fn post_sync() {
    transfers_sync::request();
}

fn post_live() {
    super::actions::enqueue_ui(
        move |mut qichi: std::pin::Pin<&mut super::qobject::Kichi>| {
            qichi.as_mut().sync_transfer_live();
        },
    );
}

fn fatal_failure(failure: &Failure) -> bool {
    matches!(
        failure,
        Failure::Auth(_) | Failure::RateLimited { .. } | Failure::Captcha { .. }
    )
}

fn resets_upload_session(failure: &Failure) -> bool {
    matches!(failure, Failure::Auth(_))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn throttle_sends_first_change_and_stage_switch() {
        let mut throttle = ProgressThrottle::default();
        assert!(throttle.should_send(UploadProgress {
            stage: UploadStage::Hashing,
            bytes: 0,
            total: 1000,
        }));
        assert!(throttle.stage_changed(), "首帧要整表同步阶段名");
        assert!(!throttle.should_send(UploadProgress {
            stage: UploadStage::Hashing,
            bytes: 1,
            total: 1000,
        }));
        assert!(!throttle.stage_changed(), "被节流的刷新不能留下阶段变更");
        assert!(throttle.should_send(UploadProgress {
            stage: UploadStage::Hashing,
            bytes: 10,
            total: 1000,
        }));
        assert!(!throttle.stage_changed(), "同阶段只走轻量进度刷新");
        assert!(throttle.should_send(UploadProgress {
            stage: UploadStage::Uploading,
            bytes: 20,
            total: 1000,
        }));
        assert!(throttle.stage_changed(), "校验转上传必须整表同步");
    }

    #[test]
    fn only_auth_failures_abort_the_remaining_upload_queue() {
        assert!(resets_upload_session(&Failure::Auth("令牌过期".into())));
        assert!(!resets_upload_session(&Failure::RateLimited {
            detail: "配额不足".into()
        }));
        assert!(!resets_upload_session(&Failure::Other("连接中断".into())));
    }

    #[test]
    fn upload_notice_group_aggregates_and_resets() {
        clear_notice_group();
        record_upload_end(&UploadEnd::Done);
        record_upload_end(&UploadEnd::Cancelled);
        let notice = take_upload_notice().expect("有成功要通知");
        assert_eq!(notice.summary, "上传结束");
        assert!(notice.body.contains("成功 1 个，取消 1 个"));
        assert!(take_upload_notice().is_none(), "取走后不能重复弹");
    }

    #[test]
    fn all_dirs_adds_missing_parents_for_retry() {
        let start = UploadStart {
            batch: 1,
            batch_cancel: Arc::new(Cancel::default()),
            parent: Some(TransferJob {
                id: "folder".to_owned(),
                name: "root".to_owned(),
                kind: kichi_core::EntryKind::Folder,
                size: 0,
                audit_flagged: false,
                sub_dir: Vec::new(),
                dir: std::path::PathBuf::new(),
                direction: transfers::TransferDirection::Upload,
                upload: None,
            }),
            files: vec![UploadFile {
                index: 0,
                job: TransferJob {
                    id: "file".to_owned(),
                    name: "a.txt".to_owned(),
                    kind: kichi_core::EntryKind::File,
                    size: 1,
                    audit_flagged: false,
                    sub_dir: vec!["root".to_owned(), "empty".to_owned()],
                    dir: std::path::PathBuf::new(),
                    direction: transfers::TransferDirection::Upload,
                    upload: None,
                },
                cancel: Arc::new(Cancel::default()),
            }]
            .into(),
            dirs: Vec::new().into(),
            upload_dirs: Vec::new().into(),
        };
        assert_eq!(
            all_dirs(&start),
            vec![
                vec!["root".to_owned()],
                vec!["root".to_owned(), "empty".to_owned()]
            ]
        );
    }
}

#[cfg(test)]
#[path = "upload_worker_mock_tests.rs"]
mod mock_tests;
