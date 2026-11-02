//! 下载调度：按源条目 id 占用 + 并发批次队列。
//!
//! 与 `downloads.rs` 分工：
//! - `downloads.rs`：QML 能看到的入口（右键菜单的「下载」、取消、打开目录）与属性刷新；
//! - 本文件：占用表、并发队列、线程启动与收尾、登出收尾（保留断点与调度槽）；
//! - `queue_collect.rs`：目录遍历与跳过分类（过深 / 读失败 / 防环）；
//! - `queue_worker.rs`：下载循环、进度节流与回投；
//! - `transfers.rs`：逐条记录（排队 / 准备中 / 下载中 / 完成 / 失败 / 取消 / 已中断），
//!   跨重启保留（`docs/adr/0014-download-history-on-disk.md`）。
//!
//! 判重键是**云端条目 id**，不是本地文件名：同一条目在排队 / 准备 / 下载中时
//! 不能再入队。查重与登记在**同一把锁**里完成——列目录那几秒入口也是通的，
//! 只在入口查布尔量会漏（2026-10-27 的 P1 并发 bug 就是这么来的）。
//!
//! 独立单文件批次最多按 `KICHI_DOWNLOAD_PARALLEL` 同时下载；文件夹批次仍独占，
//! 避免「文件夹内部 3 路 + 外面 3 个单文件」把并发翻倍。后台线程绝不碰 Qt 对象：
//! 结果一律经 `actions::enqueue_ui` 回主线程。

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use kichi_core::{EntryKind, FileEntry};

use super::queue_collect::Collected;
use super::queue_worker::{download_parallel, run_jobs, spawn_batch, Start};
use super::transfers::{self, TransferJob};
use crate::session::download::Cancel;
use crate::session::Session;

/// 队列里的一个待下载条目。
#[derive(Debug, Clone)]
pub(crate) struct Job {
    pub(crate) id: String,
    /// 在所属批次里的稳定下标；文件夹续跑时保留原行 key。
    pub(crate) index: usize,
    /// 落盘用的相对目录（文件夹下载时保留层级；单文件为空）
    pub(crate) sub_dir: Vec<String>,
    pub(crate) name: String,
    /// 官方标注的大小（0 = 未知），记录表在排队阶段就能显示大小
    pub(crate) size: u64,
    /// 列表里的审核标记：详情没下发 audit 时用来跳过无用的 5 秒重试
    pub(crate) audit_flagged: bool,
}

/// 暂停批次重新入队所需的快照。
///
/// `files = Some(..)` 表示直接复用已经展开的子文件（暂停后继续，不再读目录）；
/// `None` 表示文件夹还没开始收集，继续时重新读目录。
pub(crate) struct ResumeBatch {
    pub(crate) batch: u64,
    pub(crate) source_id: String,
    pub(crate) entry: FileEntry,
    pub(crate) dir: PathBuf,
    pub(crate) files: Option<Vec<Job>>,
    pub(crate) dirs: Vec<Vec<String>>,
    pub(crate) skipped: usize,
    /// 暂停前已经结束的子文件：续跑结束时的汇总与通知仍要算上。
    pub(crate) done: usize,
    pub(crate) failed: usize,
    pub(crate) cancelled: usize,
    pub(crate) failure: Option<String>,
}

/// 一个文件下载完的结果（只用于批次汇总；具体路径 / 原因在 `transfers`）。
pub(crate) enum JobOutcome<'a> {
    Saved,
    Failed { reason: &'a str },
    Cancelled,
}

/// 一批结束时交给界面做通知 / 状态栏的汇总。
#[derive(Debug)]
pub(crate) struct BatchReport {
    pub(crate) name: String,
    pub(crate) folder: bool,
    pub(crate) done: usize,
    pub(crate) failed: usize,
    /// 用户单独取消的文件数（整批取消走 `BatchEnd::Cancelled`，不靠这个计数）
    pub(crate) cancelled: usize,
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

/// 文件夹批次里按文件下标保存的子任务取消信号。
///
/// 子项继承整批取消；单独取消某个文件只翻它自己的标志，不影响同批其它文件。
pub(crate) type JobCancels = Arc<Mutex<HashMap<usize, Arc<Cancel>>>>;

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
    job_cancels: JobCancels,
    state: PendingState,
    done: usize,
    failed: usize,
    cancelled: usize,
    /// 被跳过的目录数（收集阶段 + 下载线程建目录失败累计）
    skipped: usize,
    /// 文件夹批次没有任何文件（空文件夹：建出目录就算完成）
    empty_folder: bool,
    /// 单文件重试时保留的层级；普通下载 / 文件夹批次为空
    sub_dir: Vec<String>,
    /// 暂停继续时直接复用的文件；普通下载为 None
    resume_files: Option<Vec<Job>>,
    /// 暂停继续时原本已经收集的目录树
    resume_dirs: Vec<Vec<String>>,
    /// 最后一条失败原因（单文件批次通知用）
    failure: Option<String>,
    /// 真的起过线程（目录收集或下载）；只有这种批次才可能在登出后还活着
    started: bool,
    /// 登出时留下收尾的批次：不再参与判重，但仍占着调度槽直到线程退出
    abandoned: bool,
}

impl Pending {
    fn folder(&self) -> bool {
        self.entry.kind == EntryKind::Folder
    }
}

/// 进程内唯一的调度状态（排队 + 正在跑的若干单文件，或一个文件夹批次）。
#[derive(Default)]
struct Scheduler {
    queue: VecDeque<Pending>,
    active: Vec<Pending>,
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
///
/// `abandoned` 批次只在等旧线程退出，不算占用：重新登录后同一个文件可以再入队。
fn is_taken(scheduler: &Scheduler, source_id: &str) -> bool {
    let same_id = |pending: &Pending| !pending.abandoned && pending.source_id == source_id;
    scheduler.active.iter().any(same_id) || scheduler.queue.iter().any(same_id)
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
        job_cancels: Arc::new(Mutex::new(HashMap::new())),
        state: PendingState::Queued,
        done: 0,
        failed: 0,
        cancelled: 0,
        skipped: 0,
        empty_folder: false,
        sub_dir,
        failure: None,
        started: false,
        abandoned: false,
        resume_files: None,
        resume_dirs: Vec::new(),
    });
}

/// 登记一批下载。同一源条目未结束时返回 `None`（调用方转去定位那一行）。
///
/// **查重与登记在同一把锁里**：连点两次也只会建一个批次。
pub(crate) fn reserve(entry: FileEntry, dir: PathBuf) -> Option<u64> {
    let source_id = entry.id.clone();
    let job = TransferJob::from_entry(&entry, dir.clone());
    with_scheduler(|scheduler| {
        if is_taken(scheduler, &source_id) || transfers::has_paused_source(&source_id) {
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
        if is_taken(scheduler, &source_id) || transfers::has_paused_source(&source_id) {
            return None;
        }
        let batch = transfers::new_batch();
        transfers::start_batch(batch, &source_id, &job);
        let entry = entry_from_job(&job);
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

pub(crate) fn entry_from_job(job: &TransferJob) -> FileEntry {
    // 只用到 id / name / kind / size / audit；其余 FileEntry 字段下载路径不读。
    FileEntry {
        id: job.id.clone(),
        name: job.name.clone(),
        kind: job.kind,
        size: job.size,
        created_at: None,
        modified_at: None,
        parent_id: String::new(),
        md5: None,
        audit_flagged: job.audit_flagged,
        starred: false,
        media: false,
        video: false,
        thumbnail_url: String::new(),
    }
}

/// 把一个暂停批次放回队尾；不新建记录，也不改记录状态。
pub(crate) fn reserve_resume(resume: ResumeBatch) -> bool {
    with_scheduler(|scheduler| {
        if scheduler
            .active
            .iter()
            .chain(scheduler.queue.iter())
            .any(|pending| pending.batch == resume.batch)
            || is_taken(scheduler, &resume.source_id)
        {
            return false;
        }
        scheduler.queue.push_back(Pending {
            batch: resume.batch,
            source_id: resume.source_id,
            entry: resume.entry,
            dir: resume.dir,
            cancel: Arc::new(Cancel::default()),
            job_cancels: Arc::new(Mutex::new(HashMap::new())),
            state: PendingState::Queued,
            done: resume.done,
            failed: resume.failed,
            cancelled: resume.cancelled,
            skipped: resume.skipped,
            empty_folder: false,
            sub_dir: Vec::new(),
            resume_files: resume.files,
            resume_dirs: resume.dirs,
            failure: resume.failure,
            started: false,
            abandoned: false,
        });
        true
    })
}

/// 这个条目是否已在排队 / 准备 / 下载中（QML 菜单文案用）。
pub(crate) fn is_pending(source_id: &str) -> bool {
    with_scheduler(|scheduler| {
        is_taken(scheduler, source_id) || transfers::has_paused_source(source_id)
    })
}

/// 线程真的起来了：从现在起，登出不能再直接丢掉这个批次。
///
/// 只在 `spawn` 成功后调用（测试里的 `next_start` 不会置位，`reset` 仍会清掉）。
pub(crate) fn mark_started(batch: u64) {
    with_scheduler(|scheduler| {
        if let Some(current) = scheduler
            .active
            .iter_mut()
            .find(|pending| pending.batch == batch)
        {
            current.started = true;
        }
    });
}

/// 当前是否有一批在准备或执行，以及排队中的批次数（属性刷新用）。
pub(crate) fn counts() -> (bool, i32) {
    with_scheduler(|scheduler| {
        // active 在准备中 / 下载中 / 登出后等旧线程退出时都存在
        let running = !scheduler.active.is_empty();
        let queued = i32::try_from(scheduler.queue.len()).unwrap_or(i32::MAX);
        (running, queued)
    })
}

/// 推进队列：填满独立单文件并发槽；文件夹批次只有等其他槽全空后才独占启动。
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
///
/// 文件夹独占是刻意的：文件夹内部已经有批内并发，再与外部单文件叠加会让
/// 实际连接数翻倍。单文件只从队首连续取，保证队列顺序不被后来的小文件插队。
fn next_start() -> Option<Start> {
    with_scheduler(|scheduler| {
        // 登出后的旧线程仍占着槽；等它们全部退出后才允许新会话开跑。
        if scheduler.active.iter().any(|pending| pending.abandoned) {
            return None;
        }
        let limit = download_parallel();
        if scheduler.active.len() >= limit {
            return None;
        }
        if scheduler.active.iter().any(|pending| pending.folder()) {
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
            let mut pending = scheduler.queue.pop_front()?;
            if pending.cancel.is_cancelled() {
                // 排队期间整批被取消：不用起线程，释放占用后看下一批
                transfers::cancel_batch(pending.batch, "");
                continue;
            }
            let folder = pending.folder();
            let resume_collected = pending.resume_files.take().map(|files| {
                let dirs = std::mem::take(&mut pending.resume_dirs);
                if folder {
                    pending.job_cancels = Arc::new(Mutex::new(
                        files
                            .iter()
                            .map(|job| (job.index, Cancel::child(pending.cancel.clone())))
                            .collect(),
                    ));
                }
                Collected {
                    files,
                    dirs,
                    skipped: pending.skipped,
                }
            });
            pending.state = if folder && resume_collected.is_none() {
                PendingState::Preparing
            } else {
                PendingState::Running
            };
            if folder && resume_collected.is_none() {
                transfers::mark_preparing(pending.batch);
            } else if !folder && resume_collected.is_none() {
                // 单文件跳过「准备中」，直接 排队中 → 下载中
                transfers::begin(pending.batch, 0);
            }
            let start = Start {
                batch: pending.batch,
                folder,
                entry: pending.entry.clone(),
                dir: pending.dir.clone(),
                cancel: pending.cancel.clone(),
                job_cancels: pending.job_cancels.clone(),
                sub_dir: pending.sub_dir.clone(),
                resume_collected,
            };
            scheduler.active.push(pending);
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
    /// 这期间被暂停：断点保留，记录已标「已暂停」，占用已释放
    Paused,
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
            .active
            .iter_mut()
            .find(|pending| pending.batch == batch)?;
        let job_cancels: JobCancels = Arc::new(Mutex::new(
            collected
                .files
                .iter()
                .map(|job| (job.index, Cancel::child(current.cancel.clone())))
                .collect(),
        ));
        current.job_cancels = job_cancels.clone();
        Some((
            current.dir.clone(),
            current.cancel.clone(),
            current.entry.name.clone(),
            current.folder(),
            job_cancels,
        ))
    });
    let Some((dir, cancel, name, folder, job_cancels)) = found else {
        return StartOutcome::NotCurrent;
    };
    // 读目录那几秒里可能刚好按了暂停 / 取消：这时不要再给总账行挂子行。
    if cancel.is_paused() {
        transfers::mark_paused(batch);
        let _ = release(batch);
        return StartOutcome::Paused;
    }
    // 取消时不要再给总账行挂子行
    // （否则记录会从「已取消」闪回「排队中」再变回来）
    if cancel.is_cancelled() {
        transfers::cancel_batch(batch, "");
        let _ = release(batch);
        return StartOutcome::Cancelled;
    }
    with_scheduler(|scheduler| {
        if let Some(current) = scheduler
            .active
            .iter_mut()
            .find(|pending| pending.batch == batch)
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
            direction: super::transfers::TransferDirection::Download,
            upload: None,
        })
        .collect();
    transfers::set_skipped(batch, collected.skipped);
    transfers::expand_folder(batch, &records, &collected.dirs);

    let my_epoch = current_epoch();
    let session = session.clone();
    let spawned = std::thread::Builder::new()
        .name("kichi-dl".into())
        .spawn(move || {
            run_jobs(
                session,
                dir,
                batch,
                collected,
                cancel,
                job_cancels,
                my_epoch,
            )
        });
    match spawned {
        Ok(_) => {
            mark_started(batch);
            StartOutcome::Started
        }
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

/// 跳过一批（收集失败 / 读完目录前被取消 / 登出后的旧线程收尾）。主线程调用。
///
/// 返回这一批的汇总（名称 / 是否文件夹 / 成败与跳过计数 / 最后失败原因），
/// 供收尾时拼系统通知。只释放这一批；后面排队的批次由随后的 [`advance`] 接手。
/// 登出留下的 `abandoned` 批次也由这里释放：epoch 已变的调用方只忽略 report。
/// 调度槽只能由线程自己的收尾回调释放，绝不因为登出提前 `take`。
pub(crate) fn release(batch: u64) -> Option<BatchReport> {
    with_scheduler(|scheduler| {
        let pending = if let Some(index) = scheduler
            .active
            .iter()
            .position(|pending| pending.batch == batch)
        {
            scheduler.active.remove(index)
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
            cancelled,
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
            cancelled,
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
            .active
            .iter_mut()
            .find(|pending| pending.batch == batch)
        {
            match outcome {
                JobOutcome::Saved => {
                    pending.done += 1;
                }
                JobOutcome::Failed { reason } => {
                    pending.failed += 1;
                    pending.failure = Some(reason.to_owned());
                }
                JobOutcome::Cancelled => {
                    pending.cancelled += 1;
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
            .active
            .iter_mut()
            .find(|pending| pending.batch == batch)
        {
            pending.skipped += extra;
        }
    });
}

/// 取消所有未结束的批次。
///
/// 排队中的还没起线程，直接放掉占用（记录由调用方统一标「已取消」）；
/// 正在跑的每一批只置取消标志，等线程自己收尾后释放。
/// 返回被直接移出队列的批次号，调用方可以立即清理它们的旧断点。
pub(crate) fn cancel_all() -> Vec<u64> {
    with_scheduler(|scheduler| {
        for current in &mut scheduler.active {
            current.cancel.cancel();
        }
        let queued: Vec<u64> = scheduler
            .queue
            .iter()
            .map(|pending| pending.batch)
            .collect();
        for pending in scheduler.queue.drain(..) {
            pending.cancel.cancel();
        }
        queued
    })
}

/// 取消一个批次时，线程是否已经起来。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CancelOutcome {
    /// 还没起线程，调用方可以立即清理断点。
    Queued,
    /// 线程会自己收尾，断点清理必须等 join 之后。
    Running,
}

/// 取消一个具体传输行。
///
/// - 顶层文件 / 文件夹总账行：取消整批（排队中的直接移出队列）；
/// - 文件夹子文件：只翻该文件自己的取消信号，同批其它文件继续。
///
/// 返回 `None` 表示调度器里已经找不到这一批（通常是与完成收尾撞车）。
pub(crate) fn cancel_transfer(
    batch: u64,
    index: usize,
    whole_batch: bool,
) -> Option<CancelOutcome> {
    with_scheduler(|scheduler| {
        if whole_batch {
            if let Some(at) = scheduler
                .queue
                .iter()
                .position(|pending| pending.batch == batch)
            {
                let pending = scheduler.queue.remove(at)?;
                pending.cancel.cancel();
                return Some(CancelOutcome::Queued);
            }
            if let Some(pending) = scheduler
                .active
                .iter()
                .find(|pending| pending.batch == batch)
            {
                pending.cancel.cancel();
                return Some(CancelOutcome::Running);
            }
            return None;
        }

        if let Some(pending) = scheduler
            .active
            .iter()
            .find(|pending| pending.batch == batch)
        {
            cancel_child(pending, index);
            return Some(CancelOutcome::Running);
        }
        if let Some(pending) = scheduler
            .queue
            .iter()
            .find(|pending| pending.batch == batch)
        {
            cancel_child(pending, index);
            return Some(CancelOutcome::Queued);
        }
        None
    })
}

fn cancel_child(pending: &Pending, index: usize) {
    let parent = pending.cancel.clone();
    let mut cancels = pending
        .job_cancels
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let cancel = cancels
        .entry(index)
        .or_insert_with(|| Cancel::child(parent))
        .clone();
    drop(cancels);
    cancel.cancel();
}

/// 暂停结果：排队中的可一步暂停，运行中的要等下载线程收尾。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PauseOutcome {
    Queued,
    Running,
    NotCurrent,
}

/// 暂停一个顶层批次。子文件级暂停不提供。
pub(crate) fn pause_transfer(batch: u64) -> PauseOutcome {
    with_scheduler(|scheduler| {
        if let Some(at) = scheduler
            .queue
            .iter()
            .position(|pending| pending.batch == batch)
        {
            let Some(pending) = scheduler.queue.remove(at) else {
                return PauseOutcome::NotCurrent;
            };
            pending.cancel.pause();
            return PauseOutcome::Queued;
        }
        if let Some(pending) = scheduler
            .active
            .iter()
            .find(|pending| pending.batch == batch)
        {
            pending.cancel.pause();
            return PauseOutcome::Running;
        }
        PauseOutcome::NotCurrent
    })
}

/// 登出：作废在途结果、保留断点，并清空排队。
///
/// 已经起过线程的 active 批次**不能直接丢掉**：它们可能还卡在 `read` 里（最长
/// 一个空闲读超时），醒来后仍会碰同一个 `.kichi-part`。这里只置 `abandon` 并
/// 保留调度槽，等线程自己的收尾回调 `release` 后才允许新批次开始。
pub(crate) fn reset() {
    // 与下载线程「最后检查 abandon + rename」互斥：登出返回后绝不会再冒出成品文件。
    let _commit = crate::session::download::commit_lock();
    bump_epoch();
    with_scheduler(|scheduler| {
        let active = std::mem::take(&mut scheduler.active);
        for mut current in active {
            current.cancel.abandon();
            if current.started {
                current.abandoned = true;
                scheduler.active.push(current);
            }
        }
        for pending in scheduler.queue.drain(..) {
            pending.cancel.cancel();
        }
    });
}

#[cfg(test)]
#[path = "queue_tests.rs"]
mod tests;
