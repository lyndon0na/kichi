//! 传输任务记录表：本次运行内的下载流水。
//!
//! 只有界面侧需要这些记录（排队顺序、失败原因、落盘路径），因此不进
//! `kichi-core`，也不落盘：退出 kichi 即清空（见 `docs/adr/0006-transfer-tasks-page.md`）。
//!
//! 排队批次只登记一条「总账行」（文件夹名），轮到读目录时才用
//! [`expand_folder`] 在它后面挂上子行 —— 主列表始终一个文件夹批次一行，
//! 几百个文件同时排队时不会把上限挤爆；子行只出现在文件夹详情页。
//!
//! 并发：下载线程直接写记录，主线程（取消 / 清空 / 登出）也写，统一走
//! [`with_records`] 的互斥锁；锁中毒时继续用内部数据，不 panic。
//! QML 模型由主线程通过 `sync_transfers()` 重建，下载线程只改记录 + 投递同步任务。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use cxx_qt_lib::QVariant;
use kichi_core::{EntryKind, FileEntry};

use super::transfers_rate::{NoteState, SpeedMeter};
use super::transfers_view::{build_snapshot, live_map_at};
use super::QVariantList;

/// 进程内最多保留的记录数（超出丢最早的已结束记录，进行中的绝不丢）。
const MAX_RECORDS: usize = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransferState {
    /// 排队中，还没轮到
    Queued,
    /// 轮到了、正在读目录结构（只有文件夹批次的首次展开）
    Preparing,
    Running,
    Done,
    Failed,
    Cancelled,
}

impl TransferState {
    pub(crate) fn is_active(self) -> bool {
        matches!(self, Self::Queued | Self::Preparing | Self::Running)
    }

    pub(crate) fn is_finished(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Cancelled)
    }

    pub(crate) fn text(self) -> &'static str {
        match self {
            Self::Queued => "排队中",
            Self::Preparing => "准备中",
            Self::Running => "下载中",
            Self::Done => "已完成",
            Self::Failed => "失败",
            Self::Cancelled => "已取消",
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Preparing => "preparing",
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
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
    /// 文件夹批次展开出来的子文件行（只出现在详情页，不进主列表）
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
}

impl Transfer {
    pub(crate) fn display_name(&self) -> String {
        self.job.display_name()
    }
}

/// 文件夹批次的总账行：主列表里的唯一一行，key 为 `batch:folder`。
///
/// 子行与单文件行都是 `EntryKind::File`；父行和第一个子行都可能是
/// `(batch, index = 0)`，所以按 `(batch, index)` 查文件时必须再过滤类型。
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
pub(crate) fn new_batch() -> u64 {
    static NEXT: OnceLock<AtomicU64> = OnceLock::new();
    NEXT.get_or_init(|| AtomicU64::new(1))
        .fetch_add(1, Ordering::SeqCst)
}

/// 整批登记。真正下载的一行一个 [`TransferJob`]（文件夹总账行只有一条）。
///
/// 排队阶段只登记传给它的行；文件夹批次轮到读目录后用 [`expand_folder`]
/// 展开成整批。
pub(crate) fn start_batch(batch: u64, source_id: &str, job: &TransferJob) {
    with_records(|records| push_job(records, batch, source_id, job));
}

/// 轮到读目录：把这一批的总账行标成「准备中」。
pub(crate) fn mark_preparing(batch: u64) {
    with_records(|records| mark_preparing_in(records, batch));
}

/// 整个文件夹批次读完了：在总账行后面挂上整批的子文件行。
///
/// 父行留在原位置（主列表一个文件夹只占一行）；子行从它后面依次插入，
/// 排队中的其他批次会自然地排在它下面。
pub(crate) fn expand_folder(batch: u64, jobs: &[TransferJob]) {
    with_records(|records| expand_folder_in(records, batch, jobs));
}

/// 一批整体没跑起来（收集失败 / 空目录 / 线程起不来）：标失败。
pub(crate) fn fail_batch(batch: u64, reason: &str) {
    with_records(|records| fail_batch_in(records, batch, reason));
}

/// 一条开始下载。
pub(crate) fn begin(batch: u64, index: usize) {
    with_records(|records| begin_in(records, batch, index));
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

/// 只清已结束的记录，进行中的保留。
pub(crate) fn clear_finished() {
    with_records(clear_finished_in);
}

/// 登出时全清（记录与文件夹详情选中都清掉）。
pub(crate) fn reset() {
    with_records(|records| {
        records.clear();
        super::transfers_detail::clear_selection();
    });
}

/// 记录表 → QML 模型 + 进行中条数（排队 + 下载中）。
pub(crate) fn snapshot() -> (QVariantList, i32) {
    with_records(|records| build_snapshot(records.as_slice()))
}

/// 是否有单文件行正在下载（心跳只在此时才有意义）。
pub(crate) fn has_running_file() -> bool {
    with_records(|records| {
        records.iter().any(|record| {
            record.state == TransferState::Running && record.job.kind == EntryKind::File
        })
    })
}

/// 正在下载那一条的实时进度（当前文件行 + 文件夹父行聚合）。
///
/// 单独成一路属性：进度每 250ms 变一次，若跟着整个模型重建，`ListView`
/// 会整列表重灌、滚动位置回到顶部（2026-10-27 实测），下载期间就没法翻看了。
/// QML 按 key 取对应行，父行与当前文件行都在这一份里。
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
        let Some(running) = records.iter().find(|record| {
            record.state == TransferState::Running && record.job.kind == EntryKind::File
        }) else {
            return list;
        };
        list.append(QVariant::from(&live_map_at(running, now)));
        if running.child {
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
fn find_file(records: &mut [Transfer], batch: u64, index: usize) -> Option<&mut Transfer> {
    records.iter_mut().find(|record| {
        record.job.kind == EntryKind::File && record.batch == batch && record.index == index
    })
}

/// 一条新记录（排队态）。`index` 在文件夹展开时由外层决定。
pub(super) fn row_from(batch: u64, index: usize, source_id: &str, job: &TransferJob) -> Transfer {
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
    }
}

fn push_job(records: &mut Vec<Transfer>, batch: u64, source_id: &str, job: &TransferJob) {
    records.push(row_from(batch, 0, source_id, job));
    trim(records);
}

/// 把整批子行挂到文件夹总账行后面（总账行留在原位，状态仍是「准备中」）。
///
/// 不删父行：主列表永远只有一条文件夹批次行；子行只在详情页展示。
/// 子行从父行后面依次插入，后面排队的批次位置不受影响。
fn expand_folder_in(records: &mut Vec<Transfer>, batch: u64, jobs: &[TransferJob]) {
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
            let mut row = row_from(batch, index, &source_id, job);
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
/// 取消后不再被后续完成事件改回（取消的优先级最高）；没有子行时不动父行。
pub(crate) fn refresh_parent_state(records: &mut [Transfer], batch: u64) {
    let Some(parent) = records
        .iter()
        .position(|record| record.batch == batch && is_folder_ledger(record))
    else {
        return;
    };
    if records[parent].state == TransferState::Cancelled {
        return;
    }

    let mut children = 0usize;
    let mut active = false;
    let mut running = false;
    let mut failed = false;
    let mut cancelled = false;
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
            TransferState::Done => {}
            TransferState::Failed => {
                failed = true;
                all_done = false;
            }
            TransferState::Cancelled => {
                cancelled = true;
                all_done = false;
            }
        }
    }
    if children == 0 {
        return;
    }

    records[parent].state = if active {
        // 读目录刚结束、子行还没开始时保持「准备中」
        if running || records[parent].state != TransferState::Preparing {
            TransferState::Running
        } else {
            TransferState::Preparing
        }
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

fn fail_batch_in(records: &mut [Transfer], batch: u64, reason: &str) {
    for record in records.iter_mut().filter(|record| record.batch == batch) {
        if record.state.is_active() {
            record.state = TransferState::Failed;
            record.detail = reason.to_owned();
        }
    }
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
        if record.state.is_active() {
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
        if record.state.is_active() {
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

/// 这个批次是否已经有子文件落盘（父行的「打开位置」只在此时可用）。
pub(crate) fn has_done_child(records: &[Transfer], batch: u64) -> bool {
    records
        .iter()
        .any(|record| record.batch == batch && record.child && record.state == TransferState::Done)
}

/// 所有记录都已结束的批次号（升序）。
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

/// 清空已结束的**整批**：正在下载的文件夹即使有已完成子行也完整保留。
fn clear_finished_in(records: &mut Vec<Transfer>) {
    let removable = finished_batches(records);
    if removable.is_empty() {
        return;
    }
    records.retain(|record| !removable.contains(&record.batch));
}

/// 超过上限时按整批淘汰最早的已结束批次；进行中的批次绝不删。
///
/// 只剩最新一个已结束批次时保留它：一个超大文件夹刚下完，
/// 不能因为记录数超限就整条从界面上消失。
fn trim(records: &mut Vec<Transfer>) {
    while records.len() > MAX_RECORDS {
        let finished = finished_batches(records);
        if finished.len() < 2 {
            break;
        }
        let oldest = finished[0];
        records.retain(|record| record.batch != oldest);
    }
}

#[cfg(test)]
#[path = "transfers_tests.rs"]
mod tests;
