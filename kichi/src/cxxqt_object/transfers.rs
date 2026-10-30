//! 传输任务记录表：下载流水（跨重启保留，见 `docs/adr/0014-download-history-on-disk.md`）。
//!
//! 只有界面侧需要这些记录（排队顺序、失败原因、落盘路径），因此不进
//! `kichi-core`；落盘与读回规则拆在 [`super::transfers_history`]。
//!
//! 排队批次只登记一条「总账行」（文件夹名），轮到读目录时才用
//! [`expand_folder`] 在它后面挂上子行 —— 统一列表始终一个文件夹批次一行，
//! 几百个文件同时排队时不会把上限挤爆；子行只在父行展开时插入同一张表。
//!
//! 并发：下载线程直接写记录，主线程（取消 / 清空 / 登出）也写，统一走
//! [`with_records`] 的互斥锁；锁中毒时继续用内部数据，不 panic。
//! QML 模型由主线程通过 `sync_transfers()` 重建，下载线程只改记录 + 投递同步任务。
//!
//! **锁顺序**：先 `records`，再 `trees`（文件夹目录树）。任何地方都按这个顺序加锁。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use cxx_qt_lib::QVariant;
use kichi_core::{EntryKind, FileEntry};

use super::transfers_rate::{NoteState, SpeedMeter};
use super::transfers_view::live_map_at;
use super::QVariantList;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransferState {
    /// 排队中，还没轮到
    Queued,
    /// 轮到了、正在读目录结构（只有文件夹批次的首次展开）
    Preparing,
    Running,
    /// 用户已请求暂停，下载线程正在收尾
    Pausing,
    /// 已暂停：保留断点，不占调度槽，可继续或取消
    Paused,
    Done,
    Failed,
    Cancelled,
    /// 上次退出时还没跑完（读回历史时标上）；不能取消、也不算结束，
    /// 但可以「重试」——重启后不自动续跑（`docs/adr/0011-resumable-downloads.md`）。
    Interrupted,
}

impl TransferState {
    /// 是否占用调度槽 / 计入侧栏「传输中」。
    ///
    /// **不含** `Paused`：暂停任务不占调度槽，也不该让侧栏一直显示在传输。
    /// **不含** `Interrupted`：那一轮下载已经不存在了。
    ///
    /// 写成显式 `match`（不用 `matches!`）：将来加状态时编译器会提醒这里漏了分支。
    pub(crate) fn is_active(self) -> bool {
        match self {
            Self::Queued | Self::Preparing | Self::Running | Self::Pausing => true,
            Self::Paused | Self::Done | Self::Failed | Self::Cancelled | Self::Interrupted => false,
        }
    }

    /// 是否还能被用户取消。
    ///
    /// `Paused` 不占调度槽，但仍保留断点并允许显式取消（取消时要删 part）。
    pub(crate) fn is_cancellable(self) -> bool {
        match self {
            Self::Queued | Self::Preparing | Self::Running | Self::Pausing | Self::Paused => true,
            Self::Done | Self::Failed | Self::Cancelled | Self::Interrupted => false,
        }
    }

    /// 能否被「清空已完成记录」/ 按上限淘汰清掉。
    ///
    /// **不**含 `Interrupted`：已中断的行是用户要看、要重试的东西，只有
    /// 「清空全部记录」和清理规则能删它。
    pub(crate) fn is_finished(self) -> bool {
        match self {
            Self::Done | Self::Failed | Self::Cancelled => true,
            Self::Queued
            | Self::Preparing
            | Self::Running
            | Self::Pausing
            | Self::Paused
            | Self::Interrupted => false,
        }
    }

    pub(crate) fn text(self) -> &'static str {
        match self {
            Self::Queued => "排队中",
            Self::Preparing => "准备中",
            Self::Running => "下载中",
            Self::Pausing => "正在暂停",
            Self::Paused => "已暂停",
            Self::Done => "已完成",
            Self::Failed => "失败",
            Self::Cancelled => "已取消",
            Self::Interrupted => "已中断",
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Preparing => "preparing",
            Self::Running => "running",
            Self::Pausing => "pausing",
            Self::Paused => "paused",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
        }
    }
}

/// 传输记录的重试种子：一行要下载什么、写到哪里。
#[derive(Debug, Clone)]
pub(crate) struct TransferJob {
    /// 云端条目 id：文件行 = 文件 id，文件夹总账行 = 文件夹 id
    pub(crate) id: String,
    /// 裸名字（不含 sub_dir）
    pub(crate) name: String,
    pub(crate) kind: EntryKind,
    /// 官方标注的大小（0 = 未知）
    pub(crate) size: u64,
    /// 列表里的审核标记（详情没下发 audit 时用来跳过无用的 5 秒重试）
    pub(crate) audit_flagged: bool,
    /// 相对下载目录的层级；单文件 / 文件夹总账行为空
    pub(crate) sub_dir: Vec<String>,
    /// 入队时的下载目录快照
    pub(crate) dir: PathBuf,
}

impl TransferJob {
    /// 从当前目录 / 搜索结果里的条目建种子（普通「下载」入口）。
    pub(crate) fn from_entry(entry: &FileEntry, dir: PathBuf) -> Self {
        Self {
            id: entry.id.clone(),
            name: entry.name.clone(),
            kind: entry.kind,
            size: entry.size,
            audit_flagged: entry.audit_flagged,
            sub_dir: Vec::new(),
            dir,
        }
    }

    /// 界面显示的路径：`剧集/第 1 集.mkv`；单文件就是裸名字。
    pub(crate) fn display_name(&self) -> String {
        if self.sub_dir.is_empty() {
            self.name.clone()
        } else {
            format!("{}/{}", self.sub_dir.join("/"), self.name)
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Transfer {
    /// 批次序号（进程内自增）
    pub(crate) batch: u64,
    /// 批内顺序，决定同批次的显示顺序
    pub(crate) index: usize,
    /// 文件夹批次展开出来的子文件行（只在父行展开时进入统一列表）
    pub(crate) child: bool,
    /// 要重下所需的一切（id / 裸名 / 类型 / 大小 / 审核标记 / 子目录 / 目录快照）
    pub(crate) job: TransferJob,
    /// 来源条目 id：同一批的每一行都一样，QML 据此定位到批次
    pub(crate) source_id: String,
    pub(crate) state: TransferState,
    pub(crate) bytes: u64,
    pub(crate) total: Option<u64>,
    /// 失败 / 取消原因（完成行的说明由 path 派生）；下载中则是重试 / 续传说明
    pub(crate) detail: String,
    /// 成功落盘的最终路径；未完成 / 失败为 None
    pub(crate) path: Option<PathBuf>,
    /// 5 秒速度窗口 + 卡住时钟（只在内存，见 `transfers_rate.rs`）
    pub(crate) speed: SpeedMeter,
    /// 下载中说明的存活状态（重试 / 续传说明恢复 5 秒后自动消失）
    pub(crate) note: NoteState,
    /// 文件夹总账行：一个文件都没有，建出目录就算完成（空文件夹）
    pub(crate) empty_folder: bool,
    /// 文件夹总账行：被跳过的目录数（过深 / 读失败 / 本地建目录失败）
    pub(crate) skipped: usize,
    /// 批次入队时刻（Unix 秒）：90 天清理与整批重试保留原时刻
    pub(crate) created_unix: i64,
    /// 进程启动序号：落盘后用来识别「上一轮没跑完的行」
    pub(crate) session: u64,
    /// 正常退出时仍在排队 / 准备 / 下载，下一轮恢复登录态后自动续跑
    pub(crate) auto_resume: bool,
}

impl Transfer {
    pub(crate) fn display_name(&self) -> String {
        self.job.display_name()
    }
}

/// 文件夹批次的目录树（`queue_collect::Collected.dirs`，相对下载根目录）。
///
/// 读目录那一刻记下，落盘时写在总账行上，读回后挂回这里。批次被清掉 / 淘汰 /
/// 删行后由 [`prune_trees`] 收走，不会留到进程退出。锁顺序：先 `records`、再 `trees`。
fn trees() -> &'static Mutex<HashMap<u64, Vec<Vec<String>>>> {
    static TREES: OnceLock<Mutex<HashMap<u64, Vec<Vec<String>>>>> = OnceLock::new();
    TREES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 文件夹目录树加锁（中毒时继续用内部数据）；`transfers_history` 写盘时也走这里。
pub(super) fn with_trees<T>(f: impl FnOnce(&mut HashMap<u64, Vec<Vec<String>>>) -> T) -> T {
    let mut guard = trees().lock().unwrap_or_else(|e| e.into_inner());
    f(&mut guard)
}

/// 读回后把落盘的目录树挂回内存（只留记录表里还在的批次）。
///
/// 写盘时要从这里取树，不挂回来的话重启后的第一次写盘就把它们丢了。
/// 调用方持有 `records` 锁（锁顺序：先 records、再 trees）。
pub(super) fn restore_trees(trees: Vec<(u64, Vec<Vec<String>>)>, records: &[Transfer]) {
    let live: Vec<u64> = records.iter().map(|record| record.batch).collect();
    let kept: Vec<(u64, Vec<Vec<String>>)> = trees
        .into_iter()
        .filter(|(batch, _)| live.contains(batch))
        .collect();
    with_trees(|map| map.extend(kept));
}

/// 丢掉记录表里已经不存在的批次的目录树。
///
/// 目录树只在内存里留到批次被永久删掉（清空已完成 / 内存上限淘汰 / 启动缩小）
/// 为止；不收的话，长会话里每下过一个文件夹就留一份到进程退出。
///
/// 不能在 `with_records` 的闭包里调用（先取 records、再取 trees）。
pub(super) fn prune_trees() {
    let live: Vec<u64> =
        with_records(|records| records.iter().map(|record| record.batch).collect());
    with_trees(|trees| trees.retain(|batch, _| live.contains(batch)));
}

/// 文件夹批次的总账行：统一列表里的顶层行，key 为 `batch:folder`。
///
/// 子行与单文件行都是 `EntryKind::File`；父行和第一个子行都可能是
/// `(batch, index = 0)`，所以按 `(batch, index)` 查文件时必须再过滤类型。
/// 空文件夹的总账行**没有子行**，`Done` 由 [`finish_folder_empty`] 直接写下。
pub(crate) fn is_folder_ledger(record: &Transfer) -> bool {
    !record.child && record.job.kind == EntryKind::Folder
}

/// 记录的稳定标识（QML 用它把实时进度贴到对应的行上，逐条动作也按它查）。
///
/// 文件夹总账行固定为 `batch:folder`，文件行 / 子行是 `batch:index`。
pub(crate) fn record_key(record: &Transfer) -> String {
    if is_folder_ledger(record) {
        format!("{}:folder", record.batch)
    } else {
        format!("{}:{}", record.batch, record.index)
    }
}

fn records() -> &'static Mutex<Vec<Transfer>> {
    static RECORDS: OnceLock<Mutex<Vec<Transfer>>> = OnceLock::new();
    RECORDS.get_or_init(|| Mutex::new(Vec::new()))
}

/// 记录表加锁（中毒时继续用内部数据）；`transfers_actions` 的逐条动作也走这里。
pub(super) fn with_records<T>(f: impl FnOnce(&mut Vec<Transfer>) -> T) -> T {
    let mut guard = records().lock().unwrap_or_else(|e| e.into_inner());
    f(&mut guard)
}

/// 取一个新的批次序号。
///
/// 启动时由 [`raise_batch_counter`] 跳到历史最大值之后，避免与读回的行撞 key。
pub(crate) fn new_batch() -> u64 {
    counter().fetch_add(1, Ordering::SeqCst)
}

fn counter() -> &'static AtomicU64 {
    static NEXT: OnceLock<AtomicU64> = OnceLock::new();
    NEXT.get_or_init(|| AtomicU64::new(1))
}

/// 批次计数器至少走到 `max_batch` 之后（读回历史时调用）。
pub(crate) fn raise_batch_counter(max_batch: u64) {
    let next = max_batch.saturating_add(1);
    counter().fetch_max(next, Ordering::SeqCst);
}

/// 这一轮运行的会话号（读回后的新行都盖上它，用来区分「上一轮没跑完」）。
fn session_slot() -> &'static AtomicU64 {
    static SESSION: OnceLock<AtomicU64> = OnceLock::new();
    SESSION.get_or_init(|| AtomicU64::new(1))
}

/// 当前运行的会话号。
pub(crate) fn current_session() -> u64 {
    session_slot().load(Ordering::SeqCst)
}

/// 设定这一轮运行的会话号（启动加载历史之后调用一次）。
pub(crate) fn set_session(session: u64) {
    session_slot().store(session, Ordering::SeqCst);
}

/// 整批登记。真正下载的一行一个 [`TransferJob`]（文件夹总账行只有一条）。
///
/// 排队阶段只登记传给它的行；文件夹批次轮到读目录后用 [`expand_folder`]
/// 展开成整批。
pub(crate) fn start_batch(batch: u64, source_id: &str, job: &TransferJob) {
    let created = super::transfers_history::now_unix();
    let session = current_session();
    with_records(|records| push_job(records, batch, source_id, job, created, session));
    // 记录数超上限时 `push_job` 会淘汰整批，被淘汰批次的目录树跟着收走
    prune_trees();
}

/// 轮到读目录：把这一批的总账行标成「准备中」。
pub(crate) fn mark_preparing(batch: u64) {
    with_records(|records| mark_preparing_in(records, batch));
}

/// 整个文件夹批次读完了：在总账行后面挂上整批的子文件行。
///
/// 父行留在原位置（统一列表一个文件夹只占一行）；子行从它后面依次插入，
/// 排队中的其他批次会自然地排在它下面。`dirs` 是相对下载根目录的目录树，
/// 随总账行落盘（读回时的孤儿校验用；整批重试会重新读目录）。
pub(crate) fn expand_folder(batch: u64, jobs: &[TransferJob], dirs: &[Vec<String>]) {
    let created = super::transfers_history::now_unix();
    let session = current_session();
    // 锁顺序：先 records、再 trees
    with_records(|records| {
        if let Some(created_unix) = records
            .iter()
            .find(|record| record.batch == batch && is_folder_ledger(record))
            .map(|record| record.created_unix)
        {
            expand_folder_in(records, batch, jobs, created_unix, session);
        } else {
            expand_folder_in(records, batch, jobs, created, session);
        }
    });
    with_trees(|trees| {
        trees.insert(batch, dirs.to_vec());
    });
    // `expand_folder_in` 也会 trim；被淘汰的批次目录树跟着收走（新批次在记录表里，留着）
    prune_trees();
}

/// 读回一条历史行（`transfers_history::restore` 用）：不跑任何状态迁移。
#[allow(clippy::too_many_arguments)]
pub(super) fn restore_row(
    batch: u64,
    index: usize,
    child: bool,
    job: &TransferJob,
    source_id: &str,
    state: TransferState,
    bytes: u64,
    total: Option<u64>,
    detail: String,
    path: Option<PathBuf>,
    empty_folder: bool,
    skipped: usize,
    created_unix: i64,
    session: u64,
    auto_resume: bool,
) -> Transfer {
    Transfer {
        batch,
        index,
        child,
        job: job.clone(),
        source_id: source_id.to_owned(),
        state,
        bytes,
        total: total.or((job.size > 0).then_some(job.size)),
        detail,
        path,
        speed: SpeedMeter::default(),
        note: NoteState::default(),
        empty_folder,
        skipped,
        created_unix,
        session,
        auto_resume,
    }
}

/// 一批整体没跑起来（收集失败 / 线程起不来 / 根目录建不出来）：标失败。
pub(crate) fn fail_batch(batch: u64, reason: &str) {
    with_records(|records| fail_batch_in(records, batch, reason));
}

/// 文件夹收集阶段记下的跳过数（展开子行之后写，空文件夹也写）。
pub(crate) fn set_skipped(batch: u64, skipped: usize) {
    with_records(|records| set_skipped_in(records, batch, skipped));
}

/// 下载阶段又跳过了几个目录（本地空目录创建失败）。
pub(crate) fn add_skipped(batch: u64, extra: usize) {
    if extra == 0 {
        return;
    }
    with_records(|records| add_skipped_in(records, batch, extra));
}

/// 空文件夹批次结束：父行直接标完成，落盘路径 = 下载目录 + 文件夹名。
///
/// 这一批没有子行，[`refresh_parent_state`] 不会碰它；取消过的行不改。
pub(crate) fn finish_folder_empty(batch: u64) {
    with_records(|records| finish_folder_empty_in(records, batch));
}

/// 一条开始下载。
pub(crate) fn begin(batch: u64, index: usize) {
    with_records(|records| begin_in(records, batch, index));
}

/// 用户点了暂停：先把仍归属于下载线程的行标成「正在暂停」。
pub(crate) fn mark_pausing(batch: u64) {
    with_records(|records| mark_pausing_in(records, batch));
}

/// 下载线程已收尾：把仍在途的行落成「已暂停」。
pub(crate) fn mark_paused(batch: u64) {
    with_records(|records| {
        mark_paused_in(records, batch);
        // 文件夹可能刚好在暂停请求到达时下完最后一个文件；父行该落成完成，
        // 不能留下「已暂停但没有可继续子项」的死状态。
        refresh_parent_state(records, batch);
    });
}

/// 排队中的批次还没起线程，可以一步进入「已暂停」。
pub(crate) fn pause_queued(batch: u64) {
    with_records(|records| {
        pause_queued_in(records, batch);
        refresh_parent_state(records, batch);
    });
}

/// 继续暂停批次：记录回到「排队中」，由调度器重新起线程。
pub(crate) fn resume_paused(batch: u64) {
    with_records(|records| resume_paused_in(records, batch));
}

/// 正常退出前给仍在排队 / 准备 / 下载的行打自动续跑标记。
///
/// 暂停、失败、取消、已中断和已完成行一律清掉，恢复时只消费明确标过的行。
pub(crate) fn mark_auto_resume() {
    with_records(|records| mark_auto_resume_in(records));
}

/// 启动时清掉自动续跑标记：必须在真正启动前写盘，避免随后崩溃又续一轮。
pub(crate) fn clear_auto_resume() {
    with_records(|records| clear_auto_resume_in(records));
}

fn mark_auto_resume_in(records: &mut [Transfer]) {
    let auto_batches: Vec<u64> = records
        .iter()
        .filter(|record| {
            !record.child
                && matches!(
                    record.state,
                    TransferState::Queued | TransferState::Preparing | TransferState::Running
                )
        })
        .map(|record| record.batch)
        .collect();
    for record in records {
        record.auto_resume = auto_batches.contains(&record.batch)
            && matches!(
                record.state,
                TransferState::Queued | TransferState::Preparing | TransferState::Running
            );
    }
}

fn clear_auto_resume_in(records: &mut [Transfer]) {
    for record in records {
        record.auto_resume = false;
    }
}

/// 进度更新（只在 Running 时接受，取消后迟到的回调不复活记录）。
pub(crate) fn progress(batch: u64, index: usize, bytes: u64, total: Option<u64>) {
    with_records(|records| progress_in(records, batch, index, bytes, total));
}

/// 下载中的状态提示（续传 / 重试原因）；只写进行中行的 detail。
///
/// 走 `post_live()` 下发（见 `queue_worker`）：它是下载中的高频说明，
/// 不属于结构变化，跟着 `transferLive` 贴到行上，不重建列表。
pub(crate) fn note(batch: u64, index: usize, text: &str) {
    with_records(|records| note_in(records, batch, index, text));
}

/// 下载成功：记下落盘路径。
pub(crate) fn finish_ok(batch: u64, index: usize, path: &Path) {
    with_records(|records| finish_ok_in(records, batch, index, path));
}

/// 下载失败：记下原因。
pub(crate) fn finish_err(batch: u64, index: usize, reason: &str) {
    with_records(|records| finish_err_in(records, batch, index, reason));
}

/// 把进行中的记录标为已取消（登出 / 用户点取消时用）。
///
/// 用户点取消时 `reason` 为空（状态文字已说明）；线程起不来时用它附带原因。
/// 取消的作用范围由调度器决定（`queue::cancel_all`），这里只负责改状态。
pub(crate) fn cancel_active(reason: &str) {
    with_records(|records| cancel_active_in(records, None, reason));
}

/// 只把某一批里进行中的记录标为已取消（后台线程收尾用）。
pub(crate) fn cancel_batch(batch: u64, reason: &str) {
    with_records(|records| cancel_active_in(records, Some(batch), reason));
}

/// 只取消文件夹批次里的一个子文件，父行按剩余子文件重新聚合。
pub(crate) fn cancel_file(batch: u64, index: usize) {
    with_records(|records| cancel_file_in(records, batch, index));
}

/// 当前所有已暂停的顶层批次。
pub(crate) fn paused_batches() -> Vec<u64> {
    with_records(|records| {
        let mut batches: Vec<u64> = records
            .iter()
            .filter(|record| !record.child && record.state == TransferState::Paused)
            .map(|record| record.batch)
            .collect();
        batches.sort_unstable();
        batches.dedup();
        batches
    })
}

/// 这个源条目是否已有一批处于暂停态。
///
/// `Pausing` 仍占调度器 active 槽，由调度器的查重负责；`Paused` 已释放调度槽，
/// 但同一云端条目仍不能再排一批，否则两批会抢同一个 `.kichi-part`。
pub(crate) fn has_paused_source(source_id: &str) -> bool {
    with_records(|records| {
        records.iter().any(|record| {
            !record.child && record.source_id == source_id && record.state == TransferState::Paused
        })
    })
}

/// 取消一个已暂停批次：未完成行全部改成已取消，并删掉它们的断点。
pub(crate) fn cancel_paused_batch(batch: u64) {
    with_records(|records| {
        for record in records.iter_mut().filter(|record| record.batch == batch) {
            if record.state != TransferState::Done {
                record.state = TransferState::Cancelled;
                record.detail.clear();
                record.speed = SpeedMeter::default();
                record.note = NoteState::default();
            }
        }
    });
    discard_batch_parts(batch);
}

/// 删掉某一批里所有未完成文件行的断点；已完成文件没有 part，不受影响。
pub(crate) fn discard_batch_parts(batch: u64) {
    discard_parts(batch, None);
}

/// 只清一个文件夹子文件在暂停 / 排队取消后的断点。
pub(crate) fn discard_file_part(batch: u64, index: usize) {
    discard_parts(batch, Some(index));
}

fn discard_parts(batch: u64, index: Option<usize>) {
    let jobs = with_records(|records| {
        records
            .iter()
            .filter(|record| {
                record.batch == batch
                    && index.is_none_or(|index| record.index == index)
                    && record.job.kind == EntryKind::File
                    && record.state != TransferState::Done
            })
            .map(|record| (record.job.id.clone(), record.job.dir.clone()))
            .collect::<Vec<_>>()
    });
    for (id, dir) in jobs {
        crate::session::partials::discard(&id, &dir);
    }
}

fn cancel_file_in(records: &mut [Transfer], batch: u64, index: usize) {
    let changed = match find_file(records, batch, index) {
        Some(record) if record.state.is_cancellable() => {
            record.state = TransferState::Cancelled;
            record.detail.clear();
            true
        }
        _ => false,
    };
    if changed {
        refresh_parent_state(records, batch);
    }
}

/// 登出时把还在排队的记录标成「已中断」：这一轮下载已经作废，不会再有结果。
///
/// **不动正在跑的那一批的状态语义**：`queue::reset()` 只置 `abandon`，旧线程退出
/// 时 `post_finished` 发现 epoch 变了就不再改记录（`ADR 0014`），所以这里可以把
/// 进行中的行直接落成「已中断」，界面不会一直挂着假的「下载中」。
pub(crate) fn interrupt_active() {
    with_records(|records| interrupt_active_in(records));
}

fn interrupt_active_in(records: &mut [Transfer]) {
    for record in records.iter_mut() {
        if matches!(
            record.state,
            TransferState::Queued
                | TransferState::Preparing
                | TransferState::Running
                | TransferState::Pausing
        ) {
            record.state = TransferState::Interrupted;
            record.detail.clear();
        }
    }
}

/// 「清空已完成记录」：清掉整张记录表里所有「所有行都已结束」的批次。
///
/// 不能像 [`clear_finished_in`] 那样只满足 `is_finished()`：已中断的行不算结束，
/// 留着它会让整批（连同里面的失败 / 完成行）一起留下，和按钮文案不符。
pub(crate) fn clear_all() {
    with_records(clear_all_in);
    prune_trees();
}

/// 登出时清空（记录、文件夹目录树与详情选中都清掉）。
///
/// **生产路径不再调用**：ADR 0014 起登出保留历史（`downloads::reset_downloads`
/// 只作废在途下载并写盘），这里留给「确实要清干净」的单测。
#[cfg(test)]
pub(crate) fn reset() {
    with_records(|records| {
        records.clear();
        super::transfers_detail::clear_selection();
    });
    with_trees(|trees| trees.clear());
}

/// 是否有单文件行正在下载（心跳只在此时才有意义）。
pub(crate) fn has_running_file() -> bool {
    with_records(|records| {
        records.iter().any(|record| {
            record.state == TransferState::Running && record.job.kind == EntryKind::File
        })
    })
}

/// 所有正在下载文件的实时进度（各文件行 + 文件夹父行聚合）。
///
/// 单独成一路属性：进度每 250ms 变一次，若跟着整个模型重建，`ListView`
/// 会整列表重灌、滚动位置回到顶部（2026-10-27 实测），下载期间就没法翻看了。
/// QML 按 key 取对应行；父行聚合和展开的子行都从这一份取速度 / 进度。
pub(crate) fn live_snapshot() -> QVariantList {
    live_snapshot_at(Instant::now())
}

/// [`live_snapshot`] 的注入时间版本（测试用；生产走 `Instant::now()`）。
///
/// 速度 / 卡住 / 说明都由 `now` 现算：卡住秒数没有进度回调也能靠主线程
/// 1Hz 心跳（[`super::downloads`] 的 `maybe_heartbeat_live`）继续走。
pub(crate) fn live_snapshot_at(now: Instant) -> QVariantList {
    with_records(|records| {
        let mut list = QVariantList::default();
        // 批内并发与跨批并发都可能同时出现多条「下载中」：每一条都要下发自己的
        // 实时进度，否则除第一条外的父行 / 子行会停在开始时的值。
        let mut parent_batches = Vec::new();
        for running in records.iter().filter(|record| {
            record.state == TransferState::Running && record.job.kind == EntryKind::File
        }) {
            list.append(QVariant::from(&live_map_at(running, now)));
            if !running.child || parent_batches.contains(&running.batch) {
                continue;
            }
            parent_batches.push(running.batch);
            if let Some(parent) = records
                .iter()
                .find(|record| record.batch == running.batch && is_folder_ledger(record))
            {
                list.append(QVariant::from(&super::transfers_view::parent_live_map_at(
                    parent, records, now,
                )));
            }
        }
        list
    })
}

/// 按 `(batch, index)` 找文件行：必须过滤掉文件夹总账行。
///
/// 父行与第一个子行都可能是 `(batch, 0)`；只看坐标会误把父行当文件。
/// 已中断的行不再收下载线程的迟到回调（那一轮下载已经不存在了）。
fn find_file(records: &mut [Transfer], batch: u64, index: usize) -> Option<&mut Transfer> {
    records.iter_mut().find(|record| {
        record.job.kind == EntryKind::File
            && record.batch == batch
            && record.index == index
            && record.state != TransferState::Interrupted
    })
}

/// 一条新记录（排队态）。`index` 在文件夹展开时由外层决定。
pub(super) fn row_from(
    batch: u64,
    index: usize,
    source_id: &str,
    job: &TransferJob,
    created_unix: i64,
    session: u64,
) -> Transfer {
    Transfer {
        batch,
        index,
        child: false,
        job: job.clone(),
        source_id: source_id.to_owned(),
        state: TransferState::Queued,
        bytes: 0,
        total: (job.size > 0).then_some(job.size),
        detail: String::new(),
        path: None,
        speed: SpeedMeter::default(),
        note: NoteState::default(),
        empty_folder: false,
        skipped: 0,
        created_unix,
        session,
        auto_resume: false,
    }
}

fn push_job(
    records: &mut Vec<Transfer>,
    batch: u64,
    source_id: &str,
    job: &TransferJob,
    created_unix: i64,
    session: u64,
) {
    records.push(row_from(batch, 0, source_id, job, created_unix, session));
    trim(records);
}

/// 把整批子行挂到文件夹总账行后面（总账行留在原位，状态仍是「准备中」）。
///
/// 不删父行：统一列表永远只有一条文件夹批次行；子行只在展开时展示。
/// 子行从父行后面依次插入，后面排队的批次位置不受影响。
fn expand_folder_in(
    records: &mut Vec<Transfer>,
    batch: u64,
    jobs: &[TransferJob],
    created_unix: i64,
    session: u64,
) {
    let Some(at) = records
        .iter()
        .position(|record| record.batch == batch && is_folder_ledger(record))
    else {
        return;
    };
    let source_id = records[at].source_id.clone();
    let rows: Vec<Transfer> = jobs
        .iter()
        .enumerate()
        .map(|(index, job)| {
            let mut row = row_from(batch, index, &source_id, job, created_unix, session);
            row.child = true;
            row
        })
        .collect();
    records.splice(at + 1..at + 1, rows);
    trim(records);
}

fn begin_in(records: &mut [Transfer], batch: u64, index: usize) {
    begin_in_at(records, batch, index, Instant::now());
}

/// [`begin_in`] 的注入时间版本（测试用）。
pub(crate) fn begin_in_at(records: &mut [Transfer], batch: u64, index: usize, now: Instant) {
    let changed = match find_file(records, batch, index) {
        Some(record) => begin_record(record, now),
        None => false,
    };
    if changed {
        refresh_parent_state(records, batch);
    }
}

/// 一条记录开始下载：状态转「下载中」，卡住时钟落在 `now`（还没收到字节也算）。
pub(crate) fn begin_record(record: &mut Transfer, now: Instant) -> bool {
    if record.state != TransferState::Queued {
        return false;
    }
    record.state = TransferState::Running;
    record.speed.begin(now);
    true
}

/// 文件夹批次的总账行状态由子行聚合而来。
///
/// 取消 / 已中断后不再被后续完成事件改回（两者的优先级最高）；没有子行时不动父行。
pub(crate) fn refresh_parent_state(records: &mut [Transfer], batch: u64) {
    let Some(parent) = records
        .iter()
        .position(|record| record.batch == batch && is_folder_ledger(record))
    else {
        return;
    };
    let parent_state = records[parent].state;
    if matches!(
        parent_state,
        TransferState::Cancelled | TransferState::Interrupted
    ) {
        return;
    }

    let mut children = 0usize;
    let mut active = false;
    let mut running = false;
    let mut pausing = false;
    let mut paused = false;
    let mut failed = false;
    let mut cancelled = false;
    let mut interrupted = false;
    let mut all_done = true;
    for child in records
        .iter()
        .filter(|record| record.batch == batch && record.child)
    {
        children += 1;
        match child.state {
            TransferState::Queued | TransferState::Preparing => {
                active = true;
                all_done = false;
            }
            TransferState::Running => {
                active = true;
                running = true;
                all_done = false;
            }
            TransferState::Pausing => {
                pausing = true;
                all_done = false;
            }
            TransferState::Paused => {
                paused = true;
                all_done = false;
            }
            TransferState::Done => {}
            TransferState::Failed => {
                failed = true;
                all_done = false;
            }
            TransferState::Cancelled => {
                cancelled = true;
                all_done = false;
            }
            // 上一轮没跑完的子行：父行保持「已中断」，不并入失败
            TransferState::Interrupted => {
                interrupted = true;
                all_done = false;
            }
        }
    }
    if children == 0 {
        return;
    }

    records[parent].state = if pausing {
        TransferState::Pausing
    } else if active {
        // 读目录刚结束、子行还没开始时保持「准备中」
        if running || parent_state != TransferState::Preparing {
            TransferState::Running
        } else {
            TransferState::Preparing
        }
    } else if paused {
        TransferState::Paused
    } else if interrupted {
        TransferState::Interrupted
    } else if failed {
        TransferState::Failed
    } else if all_done {
        TransferState::Done
    } else if cancelled {
        TransferState::Cancelled
    } else {
        TransferState::Done
    };
}

fn mark_preparing_in(records: &mut [Transfer], batch: u64) {
    // 只有文件夹总账行需要「准备中」；单文件批次直接 排队中 → 下载中
    for record in records
        .iter_mut()
        .filter(|record| record.batch == batch && is_folder_ledger(record))
    {
        if record.state == TransferState::Queued {
            record.state = TransferState::Preparing;
        }
    }
}

fn mark_pausing_in(records: &mut [Transfer], batch: u64) {
    for record in records.iter_mut().filter(|record| record.batch == batch) {
        if matches!(
            record.state,
            TransferState::Queued | TransferState::Preparing | TransferState::Running
        ) {
            record.state = TransferState::Pausing;
            record.detail.clear();
        }
    }
}

fn mark_paused_in(records: &mut [Transfer], batch: u64) {
    for record in records.iter_mut().filter(|record| record.batch == batch) {
        if matches!(
            record.state,
            TransferState::Queued
                | TransferState::Preparing
                | TransferState::Running
                | TransferState::Pausing
        ) {
            record.state = TransferState::Paused;
            record.detail.clear();
        }
    }
}

fn pause_queued_in(records: &mut [Transfer], batch: u64) {
    for record in records.iter_mut().filter(|record| record.batch == batch) {
        if matches!(
            record.state,
            TransferState::Queued | TransferState::Preparing
        ) {
            record.state = TransferState::Paused;
            record.detail.clear();
        }
    }
}

fn resume_paused_in(records: &mut [Transfer], batch: u64) {
    for record in records.iter_mut().filter(|record| record.batch == batch) {
        if matches!(
            record.state,
            TransferState::Paused | TransferState::Interrupted
        ) {
            record.state = TransferState::Queued;
            record.detail.clear();
            record.speed = SpeedMeter::default();
            record.note = NoteState::default();
        }
    }
}

fn fail_batch_in(records: &mut [Transfer], batch: u64, reason: &str) {
    for record in records.iter_mut().filter(|record| record.batch == batch) {
        if matches!(
            record.state,
            TransferState::Queued
                | TransferState::Preparing
                | TransferState::Running
                | TransferState::Pausing
        ) {
            record.state = TransferState::Failed;
            record.detail = reason.to_owned();
        }
    }
}

/// 跳过数只记在文件夹总账行上（QML 的说明行从这一行取）。
fn set_skipped_in(records: &mut [Transfer], batch: u64, skipped: usize) {
    for record in records.iter_mut().filter(|record| record.batch == batch) {
        if is_folder_ledger(record) {
            record.skipped = skipped;
        }
    }
}

fn add_skipped_in(records: &mut [Transfer], batch: u64, extra: usize) {
    for record in records.iter_mut().filter(|record| record.batch == batch) {
        if is_folder_ledger(record) {
            record.skipped += extra;
        }
    }
}

fn finish_folder_empty_in(records: &mut [Transfer], batch: u64) {
    let Some(parent) = records
        .iter_mut()
        .find(|record| record.batch == batch && is_folder_ledger(record))
    else {
        return;
    };
    if parent.state == TransferState::Cancelled || parent.state == TransferState::Interrupted {
        return;
    }
    // 路径要在改字段前算好（函数借的是记录本身）
    let path = folder_download_path(parent);
    parent.state = TransferState::Done;
    parent.empty_folder = true;
    parent.path = path;
    parent.detail.clear();
}

fn progress_in(records: &mut [Transfer], batch: u64, index: usize, bytes: u64, total: Option<u64>) {
    progress_in_at(records, batch, index, bytes, total, Instant::now());
}

/// [`progress_in`] 的注入时间版本（测试用）。
pub(crate) fn progress_in_at(
    records: &mut [Transfer],
    batch: u64,
    index: usize,
    bytes: u64,
    total: Option<u64>,
    now: Instant,
) {
    if let Some(record) = find_file(records, batch, index) {
        progress_record(record, now, bytes, total);
    }
}

/// 进行中行的字节指纹：判断「上次写盘后有没有真的走到新字节」。
///
/// 只用于进度写盘的节流决策，不参与序列化，也不需要抗碰撞：下载卡住时
/// 心跳会反复调 `sync_transfer_live()`，指纹没变就不必重写整张表。
pub(crate) fn progress_stamp_in(records: &[Transfer]) -> u64 {
    records.iter().fold(0u64, |stamp, record| {
        if record.state != TransferState::Running {
            return stamp;
        }
        stamp
            .wrapping_mul(1_000_003)
            .wrapping_add(record.bytes)
            .wrapping_add(record.index as u64)
            .wrapping_add(record.batch)
    })
}

/// 当前记录表的进度指纹（见 [`progress_stamp_in`]）。
pub(crate) fn progress_stamp() -> u64 {
    with_records(|records| progress_stamp_in(records))
}

/// 单条记录的进度：写字节数，同时喂速度窗口与说明状态。
pub(crate) fn progress_record(record: &mut Transfer, now: Instant, bytes: u64, total: Option<u64>) {
    if record.state != TransferState::Running {
        return;
    }
    record.bytes = bytes;
    record.total = total.or(record.total);
    record.speed.observe(now, bytes);
    record.note.observe(now, bytes);
}

fn note_in(records: &mut [Transfer], batch: u64, index: usize, text: &str) {
    note_in_at(records, batch, index, text, Instant::now());
}

/// [`note_in`] 的注入时间版本（测试用）：写 `detail` 的同时重置说明存活计时。
pub(crate) fn note_in_at(
    records: &mut [Transfer],
    batch: u64,
    index: usize,
    text: &str,
    now: Instant,
) {
    if let Some(record) = find_file(records, batch, index) {
        note_record(record, now, text);
    }
}

/// 单条记录的重试 / 续传说明：未恢复前一直显示，恢复后 5 秒消失。
pub(crate) fn note_record(record: &mut Transfer, now: Instant, text: &str) {
    if record.state != TransferState::Running {
        return;
    }
    record.detail = text.to_owned();
    record.note.write(now, record.bytes);
}

fn finish_ok_in(records: &mut [Transfer], batch: u64, index: usize, path: &Path) {
    // 用户点取消后，当前文件仍可能刚好下载完整：文件已在磁盘上，照实记成完成
    let changed = if let Some(record) = find_file(records, batch, index) {
        if record.state != TransferState::Done {
            record.state = TransferState::Done;
            record.bytes = record.total.unwrap_or(record.bytes).max(record.bytes);
            record.detail.clear();
            record.path = Some(path.to_path_buf());
            true
        } else {
            false
        }
    } else {
        false
    };
    if changed {
        refresh_parent_state(records, batch);
    }
}

fn finish_err_in(records: &mut [Transfer], batch: u64, index: usize, reason: &str) {
    let changed = if let Some(record) = find_file(records, batch, index) {
        if matches!(
            record.state,
            TransferState::Queued | TransferState::Preparing | TransferState::Running
        ) {
            record.state = TransferState::Failed;
            record.detail = reason.to_owned();
            true
        } else {
            false
        }
    } else {
        false
    };
    if changed {
        refresh_parent_state(records, batch);
    }
}

fn cancel_active_in(records: &mut [Transfer], batch: Option<u64>, reason: &str) {
    for record in records {
        if batch.is_some_and(|batch| record.batch != batch) {
            continue;
        }
        if record.state.is_cancellable() {
            record.state = TransferState::Cancelled;
            record.detail = reason.to_owned();
        }
    }
}

/// 文件夹总账行对应的落盘文件夹：下载目录 + 安全化后的文件夹名。
pub(crate) fn folder_download_path(record: &Transfer) -> Option<PathBuf> {
    is_folder_ledger(record).then(|| {
        record
            .job
            .dir
            .join(crate::session::download::safe_file_name(&record.job.name))
    })
}

/// 这个批次是否已经有子文件落盘（父行的「打开位置」按此给出文件夹）。
pub(crate) fn has_done_child(records: &[Transfer], batch: u64) -> bool {
    records
        .iter()
        .any(|record| record.batch == batch && record.child && record.state == TransferState::Done)
}

/// 所有行都已结束（完成 / 失败 / 取消）的批次号（升序）。
fn finished_batches(records: &[Transfer]) -> Vec<u64> {
    let mut batches: Vec<u64> = records.iter().map(|record| record.batch).collect();
    batches.sort_unstable();
    batches.dedup();
    batches.retain(|batch| {
        records
            .iter()
            .filter(|record| record.batch == *batch)
            .all(|record| record.state.is_finished())
    });
    batches
}

/// 可以按内存上限淘汰的整批：已结束，或整批只剩「已中断」。
///
/// 只用于 [`trim`]（进程内 2000 条的内存闸门）；「清空已完成记录」不碰已中断的行。
fn evictable_batches(records: &[Transfer]) -> Vec<u64> {
    let mut batches: Vec<u64> = records.iter().map(|record| record.batch).collect();
    batches.sort_unstable();
    batches.dedup();
    batches.retain(|batch| {
        records
            .iter()
            .filter(|record| record.batch == *batch)
            .all(|record| record.state.is_finished() || record.state == TransferState::Interrupted)
    });
    batches
}

/// 清空已结束的**整批**：正在下载 / 排队 / 已中断的批次完整保留。
fn clear_finished_in(records: &mut Vec<Transfer>) {
    let removable = finished_batches(records);
    if removable.is_empty() {
        return;
    }
    records.retain(|record| !removable.contains(&record.batch));
}

/// 「清空已完成记录」：清全部历史里所有「所有行都已结束」的整批。
///
/// 与 `clear_finished_in` 同一口径（进行中 / 排队 / 已中断保留）；分出来只是为了
/// 在调用点说清楚清空口径。
fn clear_all_in(records: &mut Vec<Transfer>) {
    clear_finished_in(records);
}

/// 进程内最多保留的记录数（超出丢最早的已结束记录，进行中的绝不丢）。
///
/// 落盘的历史不受这个数限制（ADR 0014：不设条数上限）。
const MAX_RECORDS: usize = 2000;

/// 超过上限时按整批淘汰最早的已结束批次；进行中的批次绝不删。
///
/// 只剩最新一个可淘汰批次时保留它：一个超大文件夹刚下完，
/// 不能因为记录数超限就整条从界面上消失。
fn trim(records: &mut Vec<Transfer>) {
    while records.len() > MAX_RECORDS {
        let evictable = evictable_batches(records);
        if evictable.len() < 2 {
            break;
        }
        let oldest = evictable[0];
        records.retain(|record| record.batch != oldest);
    }
}

#[cfg(test)]
#[path = "transfers_tests.rs"]
mod tests;
