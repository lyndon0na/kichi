//! 上传调度：进程内串行队列。
//!
//! 上传与下载使用完全独立的调度槽。每个本地文件是一条顶层传输记录；
//! 同一规范化路径在排队或运行中时不会重复入队。后台线程在启动后先取得并
//! 持有 [`kichi_core::DriveClient`]，所以登出后仍能用旧会话尽力清理残留。

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use kichi_core::EntryKind;

use super::transfers::{self, TransferDirection, TransferJob, UploadJob};
use super::upload_worker::{spawn_upload, UploadStart};
use crate::session::download::Cancel;
use crate::session::Session;

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
    job: TransferJob,
    cancel: Arc<Cancel>,
    abandoned: bool,
}

#[derive(Default)]
struct Scheduler {
    queue: VecDeque<Pending>,
    paused: Vec<Pending>,
    active: Option<Pending>,
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

fn prepare_job(
    path: &Path,
    parent_id: &str,
    parent_name: &str,
) -> Result<TransferJob, ReserveError> {
    let path = std::fs::canonicalize(path).map_err(|err| {
        ReserveError::Invalid(format!("无法读取本地文件 {}：{err}", path.display()))
    })?;
    if path.to_str().is_none() {
        return Err(ReserveError::Invalid(
            "本地文件路径不是有效的 UTF-8，无法保存重试目标".to_owned(),
        ));
    }
    let metadata = std::fs::metadata(&path).map_err(|err| {
        ReserveError::Invalid(format!("无法读取本地文件 {}：{err}", path.display()))
    })?;
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
    let name = raw_name.to_owned();
    let parent_name = parent_name.trim();
    let parent_name = if parent_name.is_empty() {
        "全部文件"
    } else {
        parent_name
    };
    let source_id = source_id_for(&path);
    Ok(TransferJob {
        id: source_id,
        name,
        kind: EntryKind::File,
        size: metadata.len(),
        audit_flagged: false,
        sub_dir: Vec::new(),
        dir: PathBuf::new(),
        direction: TransferDirection::Upload,
        upload: Some(UploadJob {
            local_path: path,
            parent_id: parent_id.trim().to_owned(),
            parent_name: parent_name.to_owned(),
        }),
    })
}

fn is_taken(scheduler: &Scheduler, source_id: &str) -> bool {
    scheduler
        .active
        .as_ref()
        .is_some_and(|pending| !pending.abandoned && pending.source_id == source_id)
        || scheduler
            .queue
            .iter()
            .any(|pending| pending.source_id == source_id)
        || scheduler
            .paused
            .iter()
            .any(|pending| pending.source_id == source_id)
}

fn reserve_job(job: TransferJob) -> Result<u64, ReserveError> {
    let source_id = job.id.clone();
    with_scheduler(|scheduler| {
        if is_taken(scheduler, &source_id) {
            return Err(ReserveError::AlreadyPending);
        }
        let batch = transfers::new_batch();
        transfers::start_batch(batch, &source_id, &job);
        scheduler.queue.push_back(Pending {
            batch,
            source_id,
            job,
            cancel: Arc::new(Cancel::default()),
            abandoned: false,
        });
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
    reserve_job(prepare_job(
        &upload.local_path,
        &upload.parent_id,
        &upload.parent_name,
    )?)
}

pub(crate) fn counts() -> (bool, i32) {
    with_scheduler(|scheduler| {
        let running = scheduler.active.is_some();
        let queued = i32::try_from(scheduler.queue.len()).unwrap_or(i32::MAX);
        (running, queued)
    })
}

pub(crate) fn is_idle() -> bool {
    with_scheduler(|scheduler| scheduler.active.is_none() && scheduler.queue.is_empty())
}

fn next_start() -> Option<UploadStart> {
    with_scheduler(|scheduler| {
        if scheduler.active.is_some() {
            return None;
        }
        loop {
            let pending = scheduler.queue.pop_front()?;
            if pending.cancel.is_cancelled() {
                transfers::cancel_batch(pending.batch, "");
                continue;
            }
            let start = UploadStart {
                batch: pending.batch,
                job: pending.job.clone(),
                cancel: pending.cancel.clone(),
            };
            scheduler.active = Some(pending);
            return Some(start);
        }
    })
}

pub(crate) struct StartFailure {
    pub(crate) name: String,
    pub(crate) reason: String,
}

/// 主线程推进上传队列。一次只会启动一个文件。
pub(crate) fn advance(session: &Arc<Session>) -> Vec<StartFailure> {
    let mut failures = Vec::new();
    loop {
        let Some(start) = next_start() else {
            return failures;
        };
        transfers::begin(start.batch, 0);
        if let Err(err) = spawn_upload(session, &start) {
            let reason = format!("无法创建上传线程：{err}");
            transfers::finish_err(start.batch, 0, &reason);
            let _ = release(start.batch);
            failures.push(StartFailure {
                name: start.job.name,
                reason,
            });
        }
    }
}

pub(crate) fn release(batch: u64) -> bool {
    with_scheduler(|scheduler| {
        if scheduler
            .active
            .as_ref()
            .is_some_and(|active| active.batch == batch)
        {
            scheduler.active = None;
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

pub(crate) fn cancel(batch: u64) -> Option<CancelOutcome> {
    with_scheduler(|scheduler| {
        if let Some(index) = scheduler
            .queue
            .iter()
            .position(|pending| pending.batch == batch)
        {
            let pending = scheduler.queue.remove(index)?;
            pending.cancel.cancel();
            return Some(CancelOutcome::Queued);
        }
        if let Some(index) = scheduler
            .paused
            .iter()
            .position(|pending| pending.batch == batch)
        {
            let pending = scheduler.paused.remove(index);
            pending.cancel.cancel();
            return Some(CancelOutcome::Queued);
        }
        if let Some(active) = scheduler
            .active
            .as_ref()
            .filter(|active| active.batch == batch)
        {
            active.cancel.cancel();
            return Some(CancelOutcome::Running);
        }
        None
    })
}

/// 取消所有排队 / 运行中的上传；返回已直接移出队列的批次。
pub(crate) fn cancel_all() -> Vec<u64> {
    with_scheduler(|scheduler| {
        if let Some(active) = scheduler.active.as_ref() {
            active.cancel.cancel();
        }
        let queued: Vec<u64> = scheduler
            .queue
            .iter()
            .chain(scheduler.paused.iter())
            .map(|pending| pending.batch)
            .collect();
        for pending in scheduler.queue.drain(..) {
            pending.cancel.cancel();
        }
        for pending in scheduler.paused.drain(..) {
            pending.cancel.cancel();
        }
        queued
    })
}

/// 登出：停止并作废这一轮上传；旧线程退出前仍占着唯一调度槽。
pub(crate) fn reset() {
    bump_epoch();
    with_scheduler(|scheduler| {
        if let Some(active) = scheduler.active.as_mut() {
            active.cancel.abandon();
            active.abandoned = true;
        }
        for pending in scheduler.queue.drain(..) {
            pending.cancel.abandon();
        }
        for pending in scheduler.paused.drain(..) {
            pending.cancel.abandon();
        }
    });
}

/// 暂停结果：只有排队中的上传能直接暂停，运行中的仍只能取消。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PauseOutcome {
    Queued,
    Running,
    NotCurrent,
}

/// 暂停一个尚未开始的上传；保留原批次与任务快照，继续时重新排队。
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
            scheduler.paused.push(pending);
            return PauseOutcome::Queued;
        }
        if scheduler
            .active
            .as_ref()
            .is_some_and(|active| active.batch == batch)
        {
            return PauseOutcome::Running;
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
        let pending = scheduler.paused.remove(index);
        scheduler.queue.push_back(pending);
        true
    })
}

#[cfg(test)]
#[path = "upload_queue_tests.rs"]
mod tests;
