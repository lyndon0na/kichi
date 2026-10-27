//! 传输任务记录表：本次运行内的下载流水。
//!
//! 只有界面侧需要这些记录（排队顺序、失败原因、落盘路径），因此不进
//! `kichi-core`，也不落盘：退出 kichi 即清空（见 `docs/adr/0006-transfer-tasks-page.md`）。
//!
//! 排队批次只登记一条「总账行」（文件夹名），轮到读目录时才用
//! [`expand_folder`] 原地展开成整批 —— 几百个文件同时排队时不会把上限挤爆。
//!
//! 并发：下载线程直接写记录，主线程（取消 / 清空 / 登出）也写，统一走
//! [`with_records`] 的互斥锁；锁中毒时继续用内部数据，不 panic。
//! QML 模型由主线程通过 `sync_transfers()` 重建，下载线程只改记录 + 投递同步任务。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use cxx_qt_lib::QVariant;

use super::transfers_view::{live_map, notice_map, record_map};
use super::QVariantList;

/// 进程内最多保留的记录数（超出丢最早的已结束记录，进行中的绝不丢）。
const MAX_RECORDS: usize = 2000;
/// 一次最多交给 QML 的行数（大文件夹下载时避免整列表反复重建拖慢界面）。
const MAX_ROWS: usize = 500;

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

#[derive(Debug, Clone)]
pub(crate) struct Transfer {
    /// 批次序号（进程内自增）
    pub(crate) batch: u64,
    /// 批内顺序，决定同批次的显示顺序
    pub(crate) index: usize,
    /// 显示路径：剧集/第 1 集.mkv
    pub(crate) name: String,
    /// 来源条目 id：同一批的每一行都一样，QML 据此定位到批次
    pub(crate) source_id: String,
    pub(crate) state: TransferState,
    pub(crate) bytes: u64,
    pub(crate) total: Option<u64>,
    /// 完成 = 落盘路径；失败 = 原因；取消可带原因
    pub(crate) detail: String,
}

fn records() -> &'static Mutex<Vec<Transfer>> {
    static RECORDS: OnceLock<Mutex<Vec<Transfer>>> = OnceLock::new();
    RECORDS.get_or_init(|| Mutex::new(Vec::new()))
}

fn with_records<T>(f: impl FnOnce(&mut Vec<Transfer>) -> T) -> T {
    let mut guard = records().lock().unwrap_or_else(|e| e.into_inner());
    f(&mut guard)
}

/// 取一个新的批次序号。
pub(crate) fn new_batch() -> u64 {
    static NEXT: OnceLock<AtomicU64> = OnceLock::new();
    NEXT.get_or_init(|| AtomicU64::new(1))
        .fetch_add(1, Ordering::SeqCst)
}

/// 整批登记。`jobs` 是（显示路径, 大小）。
///
/// 排队阶段只登记传给它的行（文件夹批次就一条总账行）；轮到读目录后用
/// [`expand_folder`] 展开成整批。
pub(crate) fn start_batch(batch: u64, source_id: &str, jobs: &[(String, u64)]) {
    with_records(|records| push_batch(records, batch, source_id, jobs));
}

/// 轮到读目录：把这一批的总账行标成「准备中」。
pub(crate) fn mark_preparing(batch: u64) {
    with_records(|records| mark_preparing_in(records, batch));
}

/// 整个文件夹批次读完了：把总账行原地换成整批的排队行。
///
/// 原地替换（不 push 到表尾）是为了保持 (batch, index) 的显示顺序：
/// 排队中的其他批次会自然地排在它下面。
pub(crate) fn expand_folder(batch: u64, jobs: &[(String, u64)]) {
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

/// 下载成功：记下落盘路径。
pub(crate) fn finish_ok(batch: u64, index: usize, path: &str) {
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

/// 登出时全清。
pub(crate) fn reset() {
    with_records(Vec::clear);
}

/// 记录表 → QML 模型 + 进行中条数（排队 + 下载中）。
pub(crate) fn snapshot() -> (QVariantList, i32) {
    with_records(|records| build_snapshot(records.as_slice()))
}

/// 正在下载那一条的实时进度（0 或 1 个元素的列表）。
///
/// 单独成一路属性：进度每 250ms 变一次，若跟着整个模型重建，`ListView`
/// 会整列表重灌、滚动位置回到顶部（2026-10-27 实测），下载期间就没法翻看了。
pub(crate) fn live_snapshot() -> QVariantList {
    with_records(|records| {
        let mut list = QVariantList::default();
        if let Some(record) = records
            .iter()
            .find(|record| record.state == TransferState::Running)
        {
            list.append(QVariant::from(&live_map(record)));
        }
        list
    })
}

fn find(records: &mut [Transfer], batch: u64, index: usize) -> Option<&mut Transfer> {
    records
        .iter_mut()
        .find(|record| record.batch == batch && record.index == index)
}

fn push_batch(records: &mut Vec<Transfer>, batch: u64, source_id: &str, jobs: &[(String, u64)]) {
    for (index, (name, size)) in jobs.iter().enumerate() {
        records.push(Transfer {
            batch,
            index,
            name: name.clone(),
            source_id: source_id.to_owned(),
            state: TransferState::Queued,
            bytes: 0,
            total: (*size > 0).then_some(*size),
            detail: String::new(),
        });
    }
    trim(records);
}

/// 把总账行原地换成整批的排队行（位置取总账行的位置）。
fn expand_folder_in(records: &mut Vec<Transfer>, batch: u64, jobs: &[(String, u64)]) {
    let Some(at) = records
        .iter()
        .position(|record| record.batch == batch && record.index == 0)
    else {
        return;
    };
    let source_id = records[at].source_id.clone();
    let rows: Vec<Transfer> = jobs
        .iter()
        .enumerate()
        .map(|(index, (name, size))| Transfer {
            batch,
            index,
            name: name.clone(),
            source_id: source_id.clone(),
            state: TransferState::Queued,
            bytes: 0,
            total: (*size > 0).then_some(*size),
            detail: String::new(),
        })
        .collect();
    // 先把旧行拿掉再插回同一位置：整批原地替换，后面排队的批次不受影响
    records.remove(at);
    records.splice(at..at, rows);
    trim(records);
}

fn begin_in(records: &mut [Transfer], batch: u64, index: usize) {
    if let Some(record) = find(records, batch, index) {
        if record.state == TransferState::Queued {
            record.state = TransferState::Running;
        }
    }
}

fn mark_preparing_in(records: &mut [Transfer], batch: u64) {
    for record in records.iter_mut().filter(|record| record.batch == batch) {
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
    if let Some(record) = find(records, batch, index) {
        if record.state == TransferState::Running {
            record.bytes = bytes;
            record.total = total.or(record.total);
        }
    }
}

fn finish_ok_in(records: &mut [Transfer], batch: u64, index: usize, path: &str) {
    // 用户点取消后，当前文件仍可能刚好下载完整：文件已在磁盘上，照实记成完成
    if let Some(record) = find(records, batch, index) {
        if record.state != TransferState::Done {
            record.state = TransferState::Done;
            record.bytes = record.total.unwrap_or(record.bytes).max(record.bytes);
            record.detail = format!("保存到 {path}");
        }
    }
}

fn finish_err_in(records: &mut [Transfer], batch: u64, index: usize, reason: &str) {
    if let Some(record) = find(records, batch, index) {
        if record.state.is_active() {
            record.state = TransferState::Failed;
            record.detail = reason.to_owned();
        }
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

fn clear_finished_in(records: &mut Vec<Transfer>) {
    records.retain(|record| !record.state.is_finished());
}

/// 超过上限时丢最早的已结束记录；全是进行中的就保持原样（绝不丢）。
fn trim(records: &mut Vec<Transfer>) {
    while records.len() > MAX_RECORDS {
        let oldest = records
            .iter()
            .enumerate()
            .filter(|(_, record)| record.state.is_finished())
            .min_by_key(|(_, record)| (record.batch, record.index))
            .map(|(index, _)| index);
        match oldest {
            Some(index) => {
                records.remove(index);
            }
            None => break,
        }
    }
}

/// 排序：进行中在前（批次内按顺序），已结束在后（新 → 旧）。
fn ordered(records: &[Transfer]) -> Vec<&Transfer> {
    let mut active: Vec<&Transfer> = records
        .iter()
        .filter(|record| record.state.is_active())
        .collect();
    let mut finished: Vec<&Transfer> = records
        .iter()
        .filter(|record| record.state.is_finished())
        .collect();
    active.sort_by_key(|record| (record.batch, record.index));
    finished.sort_by(|a, b| (b.batch, b.index).cmp(&(a.batch, a.index)));
    active.extend(finished);
    active
}

fn build_snapshot(records: &[Transfer]) -> (QVariantList, i32) {
    let total = records.len();
    let active = records
        .iter()
        .filter(|record| record.state.is_active())
        .count();
    let mut list = QVariantList::default();
    for record in ordered(records).into_iter().take(MAX_ROWS) {
        list.append(QVariant::from(&record_map(record)));
    }
    if total > MAX_ROWS {
        list.append(QVariant::from(&notice_map(total - MAX_ROWS)));
    }
    let active = i32::try_from(active).unwrap_or(i32::MAX);
    (list, active)
}

#[cfg(test)]
#[path = "transfers_tests.rs"]
mod tests;
