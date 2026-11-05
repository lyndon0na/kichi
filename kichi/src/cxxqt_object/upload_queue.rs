//! 上传调度：单文件并发 + 文件夹批次独占。
//!
//! 上传与下载使用完全独立的调度槽。单文件批次只有一条顶层记录；文件夹批次
//! 有一个父行和若干子文件行，云端目录树由 worker 先建好，再按上传并发数传文件。
//! 单文件批次最多同时运行 `KICHI_UPLOAD_PARALLEL` 个；文件夹批次必须等所有单文件
//! 槽清空后独占启动，避免批内并发和批间并发叠加。
//! 同一规范化路径在排队或运行中时不会重复入队。后台线程在启动后先取得并
//! 持有 [`kichi_core::DriveClient`]，所以登出后仍能用旧会话尽力清理残留。

use std::collections::VecDeque;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use kichi_core::EntryKind;

use super::transfers::{self, TransferDirection, TransferJob, UploadDir, UploadJob};
use super::upload_worker::{spawn_upload, UploadFile, UploadStart};
use crate::session::download::Cancel;
use crate::session::limits;
use crate::session::Session;

pub(crate) const MAX_PATH_BYTES: usize = 4095;
pub(crate) const MAX_NAME_BYTES: usize = 255;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReserveError {
    AlreadyPending,
    Invalid(String),
}

impl ReserveError {
    pub(crate) fn reason(&self) -> String {
        match self {
            Self::AlreadyPending => "这个文件已在上传队列中".to_owned(),
            Self::Invalid(reason) => reason.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CancelOutcome {
    Queued,
    Running,
}

struct Pending {
    batch: u64,
    source_id: String,
    parent: Option<TransferJob>,
    files: Arc<[UploadFile]>,
    dirs: Arc<[Vec<String>]>,
    upload_dirs: Arc<[UploadDir]>,
    batch_cancel: Arc<Cancel>,
    abandoned: bool,
}

impl Pending {
    fn folder(&self) -> bool {
        self.parent.is_some()
    }
}

#[derive(Default)]
struct Scheduler {
    queue: VecDeque<Pending>,
    paused: Vec<Pending>,
    active: Vec<Pending>,
    /// 429 / 配额 / 鉴权失败后停住，直到用户新增或重试上传。
    blocked: Option<String>,
}

fn scheduler() -> &'static Mutex<Scheduler> {
    static SCHEDULER: OnceLock<Mutex<Scheduler>> = OnceLock::new();
    SCHEDULER.get_or_init(|| Mutex::new(Scheduler::default()))
}

fn with_scheduler<T>(f: impl FnOnce(&mut Scheduler) -> T) -> T {
    let mut guard = scheduler().lock().unwrap_or_else(|err| err.into_inner());
    f(&mut guard)
}

fn epoch() -> &'static AtomicU64 {
    static EPOCH: OnceLock<AtomicU64> = OnceLock::new();
    EPOCH.get_or_init(|| AtomicU64::new(0))
}

pub(crate) fn current_epoch() -> u64 {
    epoch().load(Ordering::SeqCst)
}

fn bump_epoch() {
    epoch().fetch_add(1, Ordering::SeqCst);
}

fn source_id_for(path: &Path) -> String {
    format!("upload:{}", path.to_string_lossy())
}

pub(crate) fn folder_source_id(path: &Path) -> String {
    format!("upload-folder:{}", path.to_string_lossy())
}

fn validate_local_path(path: &Path, label: &str) -> Result<(), ReserveError> {
    if path.as_os_str().as_bytes().len() > MAX_PATH_BYTES {
        return Err(ReserveError::Invalid(format!(
            "{label}路径太长（最多 {MAX_PATH_BYTES} 字节）"
        )));
    }
    let name = path
        .file_name()
        .ok_or_else(|| ReserveError::Invalid(format!("{label}名为空")))?;
    if name.as_bytes().len() > MAX_NAME_BYTES {
        return Err(ReserveError::Invalid(format!(
            "{label}名太长（最多 {MAX_NAME_BYTES} 字节）"
        )));
    }
    Ok(())
}

pub(crate) fn prepare_job(
    path: &Path,
    parent_id: &str,
    parent_name: &str,
) -> Result<TransferJob, ReserveError> {
    let original = std::fs::symlink_metadata(path).map_err(|err| {
        ReserveError::Invalid(format!("无法读取本地文件 {}：{err}", path.display()))
    })?;
    if original.file_type().is_symlink() {
        return Err(ReserveError::Invalid("不跟随符号链接".to_owned()));
    }
    let path = std::fs::canonicalize(path).map_err(|err| {
        ReserveError::Invalid(format!("无法读取本地文件 {}：{err}", path.display()))
    })?;
    if path.to_str().is_none() {
        return Err(ReserveError::Invalid(
            "本地文件路径不是有效的 UTF-8，无法保存重试目标".to_owned(),
        ));
    }
    validate_local_path(&path, "本地文件")?;
    let metadata = std::fs::symlink_metadata(&path).map_err(|err| {
        ReserveError::Invalid(format!("无法读取本地文件 {}：{err}", path.display()))
    })?;
    if metadata.file_type().is_symlink() {
        return Err(ReserveError::Invalid("不跟随符号链接".to_owned()));
    }
    if !metadata.is_file() {
        return Err(ReserveError::Invalid("只支持上传普通文件".to_owned()));
    }
    let raw_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| ReserveError::Invalid("文件名不是有效的 UTF-8".to_owned()))?;
    if raw_name.trim().is_empty() {
        return Err(ReserveError::Invalid("文件名不能为空".to_owned()));
    }
    let parent_name = normalize_parent_name(parent_name);
    Ok(TransferJob {
        id: source_id_for(&path),
        name: raw_name.to_owned(),
        kind: EntryKind::File,
        size: metadata.len(),
        audit_flagged: false,
        sub_dir: Vec::new(),
        dir: PathBuf::new(),
        direction: TransferDirection::Upload,
        upload: Some(UploadJob {
            local_path: path,
            parent_id: parent_id.trim().to_owned(),
            parent_name,
            batch_root_id: String::new(),
            cloud_parent_id: String::new(),
        }),
    })
}

pub(crate) fn prepare_folder_job(
    path: &Path,
    parent_id: &str,
    parent_name: &str,
) -> Result<TransferJob, ReserveError> {
    let original = std::fs::symlink_metadata(path).map_err(|err| {
        ReserveError::Invalid(format!("无法读取本地文件夹 {}：{err}", path.display()))
    })?;
    if original.file_type().is_symlink() {
        return Err(ReserveError::Invalid("不跟随符号链接".to_owned()));
    }
    let path = std::fs::canonicalize(path).map_err(|err| {
        ReserveError::Invalid(format!("无法读取本地文件夹 {}：{err}", path.display()))
    })?;
    if path.to_str().is_none() {
        return Err(ReserveError::Invalid(
            "本地文件夹路径不是有效的 UTF-8，无法保存重试目标".to_owned(),
        ));
    }
    validate_local_path(&path, "本地文件夹")?;
    let metadata = std::fs::symlink_metadata(&path).map_err(|err| {
        ReserveError::Invalid(format!("无法读取本地文件夹 {}：{err}", path.display()))
    })?;
    if metadata.file_type().is_symlink() {
        return Err(ReserveError::Invalid("不跟随符号链接".to_owned()));
    }
    if !metadata.is_dir() {
        return Err(ReserveError::Invalid("选择的不是文件夹".to_owned()));
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| ReserveError::Invalid("文件夹名不是有效的 UTF-8".to_owned()))?;
    if name.trim().is_empty() {
        return Err(ReserveError::Invalid("文件夹名不能为空".to_owned()));
    }
    Ok(TransferJob {
        id: folder_source_id(&path),
        name: name.to_owned(),
        kind: EntryKind::Folder,
        size: 0,
        audit_flagged: false,
        sub_dir: Vec::new(),
        dir: PathBuf::new(),
        direction: TransferDirection::Upload,
        upload: Some(UploadJob {
            local_path: path,
            parent_id: parent_id.trim().to_owned(),
            parent_name: normalize_parent_name(parent_name),
            batch_root_id: String::new(),
            cloud_parent_id: String::new(),
        }),
    })
}

fn normalize_parent_name(parent_name: &str) -> String {
    let parent_name = parent_name.trim();
    if parent_name.is_empty() {
        "全部文件".to_owned()
    } else {
        parent_name.to_owned()
    }
}

fn is_taken_in(scheduler: &Scheduler, source_id: &str) -> bool {
    scheduler
        .active
        .iter()
        .any(|pending| !pending.abandoned && pending.source_id == source_id)
        || scheduler
            .queue
            .iter()
            .any(|pending| pending.source_id == source_id)
        || scheduler
            .paused
            .iter()
            .any(|pending| pending.source_id == source_id)
}

pub(crate) fn is_taken(source_id: &str) -> bool {
    with_scheduler(|scheduler| is_taken_in(scheduler, source_id))
}

fn accept_new_work(scheduler: &mut Scheduler) {
    scheduler.blocked = None;
}

fn new_pending(
    batch: u64,
    source_id: String,
    parent: Option<TransferJob>,
    files: Vec<UploadFile>,
    dirs: Vec<Vec<String>>,
    upload_dirs: Vec<UploadDir>,
) -> Pending {
    let batch_cancel = Arc::new(Cancel::default());
    let files: Vec<UploadFile> = files
        .into_iter()
        .map(|file| UploadFile {
            cancel: Cancel::child(Arc::clone(&batch_cancel)),
            ..file
        })
        .collect();
    Pending {
        batch,
        source_id,
        parent,
        files: files.into(),
        dirs: dirs.into(),
        upload_dirs: upload_dirs.into(),
        batch_cancel,
        abandoned: false,
    }
}

fn reserve_job(job: TransferJob) -> Result<u64, ReserveError> {
    let source_id = job.id.clone();
    with_scheduler(|scheduler| {
        if is_taken_in(scheduler, &source_id) {
            return Err(ReserveError::AlreadyPending);
        }
        accept_new_work(scheduler);
        let batch = transfers::new_batch();
        transfers::start_batch(batch, &source_id, &job);
        scheduler.queue.push_back(new_pending(
            batch,
            source_id,
            None,
            vec![UploadFile {
                index: 0,
                job: job.clone(),
                cancel: Arc::new(Cancel::default()),
            }],
            Vec::new(),
            Vec::new(),
        ));
        Ok(batch)
    })
}

pub(crate) fn reserve(
    path: &Path,
    parent_id: &str,
    parent_name: &str,
) -> Result<u64, ReserveError> {
    reserve_job(prepare_job(path, parent_id, parent_name)?)
}

pub(crate) fn reserve_retry(job: &TransferJob) -> Result<u64, ReserveError> {
    let Some(upload) = job.upload.as_ref() else {
        return Err(ReserveError::Invalid("这条记录不是上传任务".to_owned()));
    };
    if job.kind == EntryKind::Folder {
        return Err(ReserveError::Invalid(
            "上传文件夹需要重新扫描，不能按单文件重试".to_owned(),
        ));
    }
    reserve_job(prepare_job(
        &upload.local_path,
        &upload.parent_id,
        &upload.parent_name,
    )?)
}

/// 扫描完成后把一个上传文件夹批次放进调度队列。
pub(crate) fn enqueue_folder(
    batch: u64,
    parent: TransferJob,
    files: Vec<UploadFile>,
    dirs: Vec<Vec<String>>,
    upload_dirs: Vec<UploadDir>,
) -> Result<(), ReserveError> {
    let source_id = parent.id.clone();
    with_scheduler(|scheduler| {
        if is_taken_in(scheduler, &source_id) {
            return Err(ReserveError::AlreadyPending);
        }
        accept_new_work(scheduler);
        scheduler.queue.push_back(new_pending(
            batch,
            source_id,
            Some(parent),
            files,
            dirs,
            upload_dirs,
        ));
        Ok(())
    })
}

/// 把上传文件夹里的一个失败子文件重新排进原批次。
pub(crate) fn retry_child(
    batch: u64,
    index: usize,
    parent: TransferJob,
    file: TransferJob,
    dirs: Vec<Vec<String>>,
    upload_dirs: Vec<UploadDir>,
) -> Result<(), ReserveError> {
    retry_folder(batch, parent, vec![(index, file)], dirs, upload_dirs)
}

/// 把上传文件夹里的一批失败 / 中断成员重新排进原批次。
///
/// `files` 使用记录表里的原始 `index`，worker 仍按这些行回写状态；已完成的
/// 兄弟文件不在列表里，因此不会被再次上传。
pub(crate) fn retry_folder(
    batch: u64,
    parent: TransferJob,
    files: Vec<(usize, TransferJob)>,
    dirs: Vec<Vec<String>>,
    upload_dirs: Vec<UploadDir>,
) -> Result<(), ReserveError> {
    let source_id = parent.id.clone();
    with_scheduler(|scheduler| {
        if is_taken_in(scheduler, &source_id) {
            return Err(ReserveError::AlreadyPending);
        }
        accept_new_work(scheduler);
        let files = files
            .into_iter()
            .map(|(index, job)| UploadFile {
                index,
                job,
                cancel: Arc::new(Cancel::default()),
            })
            .collect();
        scheduler.queue.push_back(new_pending(
            batch,
            source_id,
            Some(parent),
            files,
            dirs,
            upload_dirs,
        ));
        Ok(())
    })
}

pub(crate) fn counts() -> (bool, i32) {
    with_scheduler(|scheduler| {
        let running = !scheduler.active.is_empty();
        let queued = i32::try_from(scheduler.queue.len()).unwrap_or(i32::MAX);
        (running, queued)
    })
}

pub(crate) fn is_idle() -> bool {
    with_scheduler(|scheduler| scheduler.active.is_empty() && scheduler.queue.is_empty())
}

fn start_from(pending: &Pending) -> UploadStart {
    UploadStart {
        batch: pending.batch,
        batch_cancel: Arc::clone(&pending.batch_cancel),
        parent: pending.parent.clone(),
        files: pending.files.clone(),
        dirs: pending.dirs.clone(),
        upload_dirs: pending.upload_dirs.clone(),
    }
}

fn refresh_paused_pending(pending: &mut Pending) {
    if let Some((parent, dirs)) = transfers::upload_folder_resume_state(pending.batch) {
        pending.parent = Some(parent);
        pending.upload_dirs = dirs.into();
    }
}

fn next_start() -> Option<UploadStart> {
    with_scheduler(|scheduler| {
        if scheduler.blocked.is_some() {
            return None;
        }
        // 登出后的旧线程仍占着槽；等它们全部退出后才允许新会话开跑。
        if scheduler.active.iter().any(|pending| pending.abandoned) {
            return None;
        }
        let limit = limits::upload_parallel();
        if scheduler.active.len() >= limit {
            return None;
        }
        if scheduler.active.iter().any(Pending::folder) {
            return None;
        }
        loop {
            if scheduler.active.len() >= limit {
                return None;
            }
            let folder_next = scheduler.queue.front().is_some_and(Pending::folder);
            if folder_next {
                if !scheduler.active.is_empty() {
                    return None;
                }
            } else if scheduler.queue.is_empty() {
                return None;
            }
            let pending = scheduler.queue.pop_front()?;
            if pending.abandoned || pending.batch_cancel.is_abandoned() {
                transfers::interrupt_active_for(TransferDirection::Upload);
                continue;
            }
            if pending.batch_cancel.is_cancelled() {
                transfers::cancel_batch(pending.batch, "");
                continue;
            }
            let start = start_from(&pending);
            scheduler.active.push(pending);
            return Some(start);
        }
    })
}

pub(crate) struct StartFailure {
    pub(crate) name: String,
    pub(crate) reason: String,
}

/// 主线程推进上传队列：填满单文件槽；文件夹批次独占启动。
pub(crate) fn advance(session: &Arc<Session>) -> Vec<StartFailure> {
    let mut failures = Vec::new();
    loop {
        let Some(start) = next_start() else {
            return failures;
        };
        if let Err(err) = spawn_upload(session, &start) {
            let reason = format!("无法创建上传线程：{err}");
            if start.parent.is_some() {
                transfers::fail_batch(start.batch, &reason);
            } else if let Some(file) = start.files.first() {
                transfers::finish_err(start.batch, file.index, &reason);
            }
            let _ = release(start.batch);
            failures.push(StartFailure {
                name: start
                    .parent
                    .as_ref()
                    .map(|parent| parent.name.clone())
                    .or_else(|| start.files.first().map(|file| file.job.name.clone()))
                    .unwrap_or_else(|| "上传任务".to_owned()),
                reason,
            });
        }
    }
}

/// 429 / 配额 / 鉴权失败：停住本轮上传队列，并让在途请求停止但保留断点。
///
/// 用户新增或重试上传时会自动解除阻塞；不在后台偷偷恢复。
pub(crate) fn halt_for_failure(reason: &str) {
    with_scheduler(|scheduler| {
        scheduler.blocked = Some(reason.to_owned());
        for pending in &scheduler.active {
            pending.batch_cancel.halt();
        }
    });
}

/// 批次是否还在排队 / 暂停 / 正在收尾（取消后的记录等 worker 退出再删）。
pub(crate) fn is_batch_busy(batch: u64) -> bool {
    with_scheduler(|scheduler| {
        scheduler.queue.iter().any(|pending| pending.batch == batch)
            || scheduler
                .paused
                .iter()
                .any(|pending| pending.batch == batch)
            || scheduler
                .active
                .iter()
                .any(|pending| pending.batch == batch)
    })
}

pub(crate) fn release(batch: u64) -> bool {
    with_scheduler(|scheduler| {
        if let Some(index) = scheduler
            .active
            .iter()
            .position(|active| active.batch == batch)
        {
            scheduler.active.remove(index);
            return true;
        }
        if let Some(index) = scheduler
            .queue
            .iter()
            .position(|pending| pending.batch == batch)
        {
            scheduler.queue.remove(index);
            return true;
        }
        false
    })
}

/// worker 收到暂停并已收尾：把仍保留的文件快照从 active 移到 paused。
///
/// 目录 id 可能是在本轮 worker 里刚创建的，所以移动前先从记录表刷新一次；
/// 否则继续时会重复创建目录。
pub(crate) fn finish_paused(batch: u64) -> bool {
    with_scheduler(|scheduler| {
        let Some(index) = scheduler
            .active
            .iter()
            .position(|active| active.batch == batch)
        else {
            return false;
        };
        let mut pending = scheduler.active.remove(index);
        if !pending.batch_cancel.is_paused() {
            return false;
        }
        refresh_paused_pending(&mut pending);
        scheduler.paused.push(pending);
        true
    })
}

pub(crate) fn cancel(batch: u64) -> Option<CancelOutcome> {
    with_scheduler(|scheduler| {
        if let Some(index) = scheduler
            .queue
            .iter()
            .position(|pending| pending.batch == batch)
        {
            let pending = scheduler.queue.remove(index)?;
            pending.batch_cancel.cancel();
            return Some(CancelOutcome::Queued);
        }
        if let Some(index) = scheduler
            .paused
            .iter()
            .position(|pending| pending.batch == batch)
        {
            let pending = scheduler.paused.remove(index);
            pending.batch_cancel.cancel();
            return Some(CancelOutcome::Queued);
        }
        if let Some(active) = scheduler.active.iter().find(|active| active.batch == batch) {
            active.batch_cancel.cancel();
            return Some(CancelOutcome::Running);
        }
        None
    })
}

/// 取消上传文件夹里的一个子文件；整批的取消信号会继续传递给其它文件。
pub(crate) fn cancel_file(batch: u64, index: usize) -> bool {
    with_scheduler(|scheduler| {
        let find = |pending: &Pending| -> Option<Arc<Cancel>> {
            pending
                .files
                .iter()
                .find(|file| file.index == index)
                .map(|file| Arc::clone(&file.cancel))
        };
        if let Some(cancel) = scheduler
            .active
            .iter()
            .find(|pending| pending.batch == batch)
            .and_then(find)
        {
            cancel.cancel();
            return true;
        }
        if let Some(cancel) = scheduler
            .queue
            .iter()
            .find(|pending| pending.batch == batch)
            .and_then(find)
        {
            cancel.cancel();
            return true;
        }
        if let Some(cancel) = scheduler
            .paused
            .iter()
            .find(|pending| pending.batch == batch)
            .and_then(find)
        {
            cancel.cancel();
            return true;
        }
        false
    })
}

/// 取消所有排队 / 运行中的上传；返回已直接移出队列的批次。
pub(crate) fn cancel_all() -> Vec<u64> {
    with_scheduler(|scheduler| {
        for active in &scheduler.active {
            active.batch_cancel.cancel();
        }
        let queued: Vec<u64> = scheduler
            .queue
            .iter()
            .chain(scheduler.paused.iter())
            .map(|pending| pending.batch)
            .collect();
        for pending in scheduler.queue.drain(..) {
            pending.batch_cancel.cancel();
        }
        for pending in scheduler.paused.drain(..) {
            pending.batch_cancel.cancel();
        }
        queued
    })
}

/// 登出：停止并作废这一轮上传；旧线程退出前仍占着调度槽。
pub(crate) fn reset() {
    bump_epoch();
    with_scheduler(|scheduler| {
        for active in &mut scheduler.active {
            active.batch_cancel.abandon();
            active.abandoned = true;
        }
        for pending in scheduler.queue.drain(..) {
            pending.batch_cancel.abandon();
        }
        for pending in scheduler.paused.drain(..) {
            pending.batch_cancel.abandon();
        }
        scheduler.blocked = None;
    });
}

/// 暂停结果：排队项直接暂停，运行项进入「正在暂停」，等 worker 收尾。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PauseOutcome {
    Queued,
    Pausing,
    NotCurrent,
}

/// 暂停上传：排队项直接移入暂停表；运行项发协作停止信号，等 worker 收尾。
pub(crate) fn pause(batch: u64) -> PauseOutcome {
    with_scheduler(|scheduler| {
        if let Some(index) = scheduler
            .queue
            .iter()
            .position(|pending| pending.batch == batch)
        {
            let Some(pending) = scheduler.queue.remove(index) else {
                return PauseOutcome::NotCurrent;
            };
            pending.batch_cancel.pause();
            scheduler.paused.push(pending);
            return PauseOutcome::Queued;
        }
        if let Some(active) = scheduler.active.iter().find(|active| active.batch == batch) {
            active.batch_cancel.pause();
            return PauseOutcome::Pausing;
        }
        PauseOutcome::NotCurrent
    })
}

/// 继续一个已暂停的上传：复用原批次，重新排到队尾。
pub(crate) fn resume(batch: u64) -> bool {
    with_scheduler(|scheduler| {
        let Some(index) = scheduler
            .paused
            .iter()
            .position(|pending| pending.batch == batch)
        else {
            return false;
        };
        accept_new_work(scheduler);
        let mut pending = scheduler.paused.remove(index);
        pending.batch_cancel.resume();
        refresh_paused_pending(&mut pending);
        scheduler.queue.push_back(pending);
        true
    })
}

/// 一个上传批次是否还有可启动的文件（测试与 worker 诊断用）。
#[cfg(test)]
pub(crate) fn pending_file_count(batch: u64) -> usize {
    with_scheduler(|scheduler| {
        scheduler
            .active
            .iter()
            .find(|pending| pending.batch == batch)
            .or_else(|| {
                scheduler
                    .queue
                    .iter()
                    .find(|pending| pending.batch == batch)
            })
            .or_else(|| {
                scheduler
                    .paused
                    .iter()
                    .find(|pending| pending.batch == batch)
            })
            .map(|pending| pending.files.len())
            .unwrap_or(0)
    })
}

/// 记录表里某个批次的文件是否都已结束（测试与 UI 帮助函数用）。
#[cfg(test)]
pub(crate) fn batch_all_finished(batch: u64) -> bool {
    transfers::with_records(|records| {
        records
            .iter()
            .filter(|record| record.batch == batch && record.child)
            .all(|record| record.state.is_finished())
    })
}

/// 运行中的文件数（测试帮助）。
#[cfg(test)]
pub(crate) fn running_file_count(batch: u64) -> usize {
    transfers::with_records(|records| {
        records
            .iter()
            .filter(|record| {
                record.batch == batch
                    && record.child
                    && record.job.is_upload()
                    && record.state == transfers::TransferState::Running
            })
            .count()
    })
}

fn _assert_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Pending>();
}

#[cfg(test)]
#[path = "upload_queue_tests.rs"]
mod tests;
