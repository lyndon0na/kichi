//! 下载调度：按源条目 id 占用 + 串行批次队列。
//!
//! 与 `downloads.rs` 分工：
//! - `downloads.rs`：QML 能看到的入口（右键菜单的「下载」、取消、打开目录）与属性刷新；
//! - 本文件：占用表、串行队列、线程启动与收尾、登出作废；
//! - `queue_collect.rs`：目录遍历与跳过分类（过深 / 读失败 / 防环）；
//! - `queue_worker.rs`：下载循环、进度节流与回投；
//! - `transfers.rs`：本次运行内的逐条记录（排队 / 准备中 / 下载中 / 完成 / 失败 / 取消）。
//!
//! 判重键是**云端条目 id**，不是本地文件名：同一条目在排队 / 准备 / 下载中时
//! 不能再入队。查重与登记在**同一把锁**里完成——列目录那几秒入口也是通的，
//! 只在入口查布尔量会漏（2026-10-27 的 P1 并发 bug 就是这么来的）。
//!
//! 不同目录可以同时排队，但**一次只有一批在动**：轮到才读目录，读完再进下载循环。
//! 后台线程绝不碰 Qt 对象：结果一律经 `actions::enqueue_ui` 回主线程。

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use kichi_core::{EntryKind, FileEntry};

use super::queue_collect::Collected;
use super::queue_worker::{run_jobs, spawn_batch, Start};
use super::transfers::{self, TransferJob};
use crate::session::download::Cancel;
use crate::session::Session;

/// 队列里的一个待下载条目。
#[derive(Debug, Clone)]
pub(crate) struct Job {
    pub(crate) id: String,
    /// 落盘用的相对目录（文件夹下载时保留层级；单文件为空）
    pub(crate) sub_dir: Vec<String>,
    pub(crate) name: String,
    /// 官方标注的大小（0 = 未知），记录表在排队阶段就能显示大小
    pub(crate) size: u64,
    /// 列表里的审核标记：详情没下发 audit 时用来跳过无用的 5 秒重试
    pub(crate) audit_flagged: bool,
}

/// 一个文件下载完的结果（只用于批次汇总；具体路径 / 原因在 `transfers`）。
pub(crate) enum JobOutcome<'a> {
    Saved,
    Failed { reason: &'a str },
}

/// 一批结束时交给界面做通知 / 状态栏的汇总。
#[derive(Debug)]
pub(crate) struct BatchReport {
    pub(crate) name: String,
    pub(crate) folder: bool,
    pub(crate) done: usize,
    pub(crate) failed: usize,
    /// 被跳过的目录数（过深 / 云端读失败 / 本地建目录失败）
    pub(crate) skipped: usize,
    /// 文件夹批次一个文件都没有，建完目录树就算完成
    pub(crate) empty_folder: bool,
    /// 最后一条失败原因（单文件批次通知用）
    pub(crate) failure: Option<String>,
}

/// 批次还没开始就失败（读目录 / 线程起不来）。
#[derive(Debug)]
pub(crate) struct BatchFailure {
    pub(crate) name: String,
    pub(crate) folder: bool,
    pub(crate) reason: String,
}

/// 一批在调度器里的阶段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingState {
    /// 排队中，还没轮到
    Queued,
    /// 轮到了、正在读目录结构（只可能是文件夹批次）
    Preparing,
    /// 正在下载（单文件批次跳过 Preparing，直接到这里）
    Running,
}

/// 调度器里的一批下载。
struct Pending {
    /// 传输记录批号（`transfers::new_batch`）
    batch: u64,
    /// 判重键：云端条目 id
    source_id: String,
    /// 轮到时才用：文件夹要递归读目录
    entry: FileEntry,
    /// 点下时的下载目录快照（之后改设置不影响已排队的批次）
    dir: PathBuf,
    cancel: Arc<Cancel>,
    state: PendingState,
    done: usize,
    failed: usize,
    /// 被跳过的目录数（收集阶段 + 下载线程建目录失败累计）
    skipped: usize,
    /// 文件夹批次没有任何文件（空文件夹：建出目录就算完成）
    empty_folder: bool,
    /// 单文件重试时保留的层级；普通下载 / 文件夹批次为空
    sub_dir: Vec<String>,
    /// 最后一条失败原因（单文件批次通知用）
    failure: Option<String>,
}

impl Pending {
    fn folder(&self) -> bool {
        self.entry.kind == EntryKind::Folder
    }
}

/// 进程内唯一的调度状态（排队 + 正在跑的那一批）。
#[derive(Default)]
struct Scheduler {
    queue: VecDeque<Pending>,
    current: Option<Pending>,
}

fn scheduler() -> &'static Mutex<Scheduler> {
    static SCHEDULER: OnceLock<Mutex<Scheduler>> = OnceLock::new();
    SCHEDULER.get_or_init(|| Mutex::new(Scheduler::default()))
}

/// 在调度状态上加锁（中毒时继续用内部数据：这里只是队列与计数）。
fn with_scheduler<T>(f: impl FnOnce(&mut Scheduler) -> T) -> T {
    let mut guard = scheduler().lock().unwrap_or_else(|e| e.into_inner());
    f(&mut guard)
}

/// 登出代数：在途结果用它判断是否已过时。
fn epoch() -> &'static AtomicU64 {
    static EPOCH: OnceLock<AtomicU64> = OnceLock::new();
    EPOCH.get_or_init(|| AtomicU64::new(0))
}

/// 当前登出代数（主线程回投时比对）。
pub(crate) fn current_epoch() -> u64 {
    epoch().load(Ordering::SeqCst)
}

/// 登录态变化时作废在途下载。
pub(crate) fn bump_epoch() {
    epoch().fetch_add(1, Ordering::SeqCst);
}

/// 这个条目是否已有一批在排队 / 准备 / 下载中（`reserve` 与 `reserve_retry` 共用）。
fn is_taken(scheduler: &Scheduler, source_id: &str) -> bool {
    let same_id = |pending: &Pending| pending.source_id == source_id;
    scheduler.current.as_ref().is_some_and(same_id) || scheduler.queue.iter().any(same_id)
}

/// 把一个待下载条目放进队列（记录登记之后调用）。
fn enqueue(
    scheduler: &mut Scheduler,
    batch: u64,
    source_id: String,
    entry: FileEntry,
    dir: PathBuf,
    sub_dir: Vec<String>,
) {
    scheduler.queue.push_back(Pending {
        batch,
        source_id,
        entry,
        dir,
        cancel: Arc::new(Cancel::default()),
        state: PendingState::Queued,
        done: 0,
        failed: 0,
        skipped: 0,
        empty_folder: false,
        sub_dir,
        failure: None,
    });
}

/// 登记一批下载。同一源条目未结束时返回 `None`（调用方转去定位那一行）。
///
/// **查重与登记在同一把锁里**：连点两次也只会建一个批次。
pub(crate) fn reserve(entry: FileEntry, dir: PathBuf) -> Option<u64> {
    let source_id = entry.id.clone();
    let job = TransferJob::from_entry(&entry, dir.clone());
    with_scheduler(|scheduler| {
        if is_taken(scheduler, &source_id) {
            return None;
        }
        let batch = transfers::new_batch();
        // 排队批次只登记一条总账行：文件夹轮到时才在它后面挂上子行，
        // 免得几百个文件排队时把 MAX_RECORDS 挤爆。
        transfers::start_batch(batch, &source_id, &job);
        enqueue(scheduler, batch, source_id, entry, dir, Vec::new());
        Some(batch)
    })
}

/// 重试一条失败记录：与 [`reserve`] 共用同一把锁里的查重 / 登记。
/// 判重键是 `job.id`；同一文件已在排队 / 下载中时返回 `None`。
pub(crate) fn reserve_retry(job: TransferJob) -> Option<u64> {
    let source_id = job.id.clone();
    with_scheduler(|scheduler| {
        if is_taken(scheduler, &source_id) {
            return None;
        }
        let batch = transfers::new_batch();
        transfers::start_batch(batch, &source_id, &job);
        // 只用到 id / name / kind / size / audit；其余 FileEntry 字段下载路径不读。
        let entry = FileEntry {
            id: job.id.clone(),
            name: job.name.clone(),
            kind: job.kind,
            size: job.size,
            modified_at: None,
            md5: None,
            audit_flagged: job.audit_flagged,
            media: false,
        };
        enqueue(
            scheduler,
            batch,
            source_id,
            entry,
            job.dir.clone(),
            job.sub_dir.clone(),
        );
        Some(batch)
    })
}

/// 这个条目是否已在排队 / 准备 / 下载中（QML 菜单文案用）。
pub(crate) fn is_pending(source_id: &str) -> bool {
    with_scheduler(|scheduler| is_taken(scheduler, source_id))
}

/// 当前是否有一批在准备或执行，以及排队中的批次数（属性刷新用）。
pub(crate) fn counts() -> (bool, i32) {
    with_scheduler(|scheduler| {
        // `current` 只会在准备中 / 下载中两种状态下存在
        let running = scheduler.current.is_some();
        let queued = i32::try_from(scheduler.queue.len()).unwrap_or(i32::MAX);
        (running, queued)
    })
}

/// 推进队列：没有正在跑的批次时，pop 下一批并起线程。
///
/// 返回启动失败的批次（线程起不来这类必须显式收尾并通知，不能像
/// `run_in_background` 那样只打 stderr）。主线程调用。
pub(crate) fn advance(session: &Arc<Session>) -> Vec<BatchFailure> {
    let mut failures = Vec::new();
    loop {
        let Some(start) = next_start() else {
            return failures;
        };
        if let Err(err) = spawn_batch(session, &start) {
            let reason = format!("无法创建下载线程：{err}");
            transfers::fail_batch(start.batch, &reason);
            let _ = release(start.batch);
            failures.push(BatchFailure {
                name: start.entry.name.clone(),
                folder: start.folder,
                reason,
            });
        }
    }
}

/// 取下一批，并在同一个锁里标成「准备中 / 下载中」。
fn next_start() -> Option<Start> {
    with_scheduler(|scheduler| {
        if scheduler.current.is_some() {
            return None;
        }
        loop {
            let mut pending = scheduler.queue.pop_front()?;
            if pending.cancel.is_cancelled() {
                // 排队期间整批被取消：不用起线程，释放占用后看下一批
                transfers::cancel_batch(pending.batch, "");
                continue;
            }
            let folder = pending.folder();
            pending.state = if folder {
                PendingState::Preparing
            } else {
                PendingState::Running
            };
            if folder {
                transfers::mark_preparing(pending.batch);
            } else {
                // 单文件跳过「准备中」，直接 排队中 → 下载中
                transfers::begin(pending.batch, 0);
            }
            let start = Start {
                batch: pending.batch,
                folder,
                entry: pending.entry.clone(),
                dir: pending.dir.clone(),
                cancel: pending.cancel.clone(),
                sub_dir: pending.sub_dir.clone(),
            };
            scheduler.current = Some(pending);
            return Some(start);
        }
    })
}

/// 目录收集完成后启动下载的结果。
pub(crate) enum StartOutcome {
    /// 下载线程已在跑
    Started,
    /// 这一批已经不是当前批次（登出 / 已收尾）
    NotCurrent,
    /// 这期间被取消：记录已标「已取消」，占用已释放
    Cancelled,
    /// 线程起不来：记录已标失败，占用已释放
    Failed(BatchFailure),
}

/// 目录收集完成：把整批子行挂到总账行后面并启动下载线程。主线程调用。
pub(crate) fn start_download(
    session: &Arc<Session>,
    batch: u64,
    collected: Collected,
) -> StartOutcome {
    let found = with_scheduler(|scheduler| {
        let current = scheduler
            .current
            .as_mut()
            .filter(|pending| pending.batch == batch)?;
        Some((
            current.dir.clone(),
            current.cancel.clone(),
            current.entry.name.clone(),
            current.folder(),
        ))
    });
    let Some((dir, cancel, name, folder)) = found else {
        return StartOutcome::NotCurrent;
    };
    // 读目录那几秒里可能刚好按了取消：这时不要再给总账行挂子行
    // （否则记录会从「已取消」闪回「排队中」再变回来）
    if cancel.is_cancelled() {
        transfers::cancel_batch(batch, "");
        let _ = release(batch);
        return StartOutcome::Cancelled;
    }
    with_scheduler(|scheduler| {
        if let Some(current) = scheduler
            .current
            .as_mut()
            .filter(|pending| pending.batch == batch)
        {
            current.state = PendingState::Running;
            current.skipped = collected.skipped;
            current.empty_folder = collected.files.is_empty();
        }
    });

    let records: Vec<TransferJob> = collected
        .files
        .iter()
        .map(|job| TransferJob {
            id: job.id.clone(),
            name: job.name.clone(),
            kind: EntryKind::File,
            size: job.size,
            audit_flagged: job.audit_flagged,
            sub_dir: job.sub_dir.clone(),
            dir: dir.clone(),
        })
        .collect();
    transfers::set_skipped(batch, collected.skipped);
    transfers::expand_folder(batch, &records);

    let my_epoch = current_epoch();
    let session = session.clone();
    let spawned = std::thread::Builder::new()
        .name("kichi-dl".into())
        .spawn(move || run_jobs(session, dir, batch, collected, cancel, my_epoch));
    match spawned {
        Ok(_) => StartOutcome::Started,
        Err(err) => {
            let reason = format!("无法创建下载线程：{err}");
            transfers::fail_batch(batch, &reason);
            let _ = release(batch);
            StartOutcome::Failed(BatchFailure {
                name,
                folder,
                reason,
            })
        }
    }
}

/// 跳过一批（收集失败 / 读完目录前被取消）。主线程调用。
///
/// 返回这一批的汇总（名称 / 是否文件夹 / 成败与跳过计数 / 最后失败原因），
/// 供收尾时拼系统通知。只释放这一批；后面排队的批次由随后的 [`advance`] 接手。
pub(crate) fn release(batch: u64) -> Option<BatchReport> {
    with_scheduler(|scheduler| {
        let pending = if scheduler
            .current
            .as_ref()
            .is_some_and(|pending| pending.batch == batch)
        {
            scheduler.current.take()?
        } else {
            // 测试里会直接释放还没轮到、也没起线程的批次
            let index = scheduler
                .queue
                .iter()
                .position(|pending| pending.batch == batch)?;
            scheduler.queue.remove(index)?
        };
        let Pending {
            entry,
            done,
            failed,
            skipped,
            empty_folder,
            failure,
            ..
        } = pending;
        Some(BatchReport {
            name: entry.name,
            folder: entry.kind == EntryKind::Folder,
            done,
            failed,
            skipped,
            empty_folder,
            failure,
        })
    })
}

/// 记一条文件的结果（下载线程调用；只影响收尾汇总）。
pub(crate) fn add_result(batch: u64, outcome: JobOutcome<'_>) {
    with_scheduler(|scheduler| {
        if let Some(pending) = scheduler
            .current
            .as_mut()
            .filter(|pending| pending.batch == batch)
        {
            match outcome {
                JobOutcome::Saved => {
                    pending.done += 1;
                }
                JobOutcome::Failed { reason } => {
                    pending.failed += 1;
                    pending.failure = Some(reason.to_owned());
                }
            }
        }
    });
}

/// 一批里又多跳过了几个目录（下载线程建本地目录失败时调用）。
pub(crate) fn add_skipped(batch: u64, extra: usize) {
    if extra == 0 {
        return;
    }
    with_scheduler(|scheduler| {
        if let Some(pending) = scheduler
            .current
            .as_mut()
            .filter(|pending| pending.batch == batch)
        {
            pending.skipped += extra;
        }
    });
}

/// 取消所有未结束的批次。
///
/// 排队中的还没起线程，直接放掉占用（记录由调用方统一标「已取消」）；
/// 正在跑的那一批只置取消标志，等线程自己收尾后释放。
pub(crate) fn cancel_all() {
    with_scheduler(|scheduler| {
        if let Some(current) = scheduler.current.as_mut() {
            current.cancel.cancel();
        }
        for pending in scheduler.queue.drain(..) {
            pending.cancel.cancel();
        }
    });
}

/// 登出：清空调度器并作废在途结果。
pub(crate) fn reset() {
    bump_epoch();
    with_scheduler(|scheduler| {
        if let Some(current) = scheduler.current.take() {
            current.cancel.cancel();
        }
        for pending in scheduler.queue.drain(..) {
            pending.cancel.cancel();
        }
    });
}

#[cfg(test)]
#[path = "queue_tests.rs"]
mod tests;
