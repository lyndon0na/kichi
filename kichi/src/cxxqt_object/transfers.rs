//! 传输任务记录表：本次运行内的下载流水。
//!
//! 只有界面侧需要这些记录（排队顺序、失败原因、落盘路径），因此不进
//! `kichi-core`，也不落盘：退出 kichi 即清空（见 `docs/adr/0006-transfer-tasks-page.md`）。
//!
//! 并发：下载线程直接写记录，主线程（取消 / 清空 / 登出）也写，统一走
//! [`with_records`] 的互斥锁；锁中毒时继续用内部数据，不 panic。
//! QML 模型由主线程通过 `sync_transfers()` 重建，下载线程只改记录 + 投递同步任务。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use cxx_qt_lib::{QMap, QMapPair_QString_QVariant, QString, QVariant};

use super::actions::format_size;
use super::QVariantList;

type QVariantMap = QMap<QMapPair_QString_QVariant>;

/// 进程内最多保留的记录数（超出丢最早的已结束记录，进行中的绝不丢）。
const MAX_RECORDS: usize = 2000;
/// 一次最多交给 QML 的行数（大文件夹下载时避免整列表反复重建拖慢界面）。
const MAX_ROWS: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransferState {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
}

impl TransferState {
    fn is_active(self) -> bool {
        matches!(self, Self::Queued | Self::Running)
    }

    fn is_finished(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Cancelled)
    }

    fn text(self) -> &'static str {
        match self {
            Self::Queued => "排队中",
            Self::Running => "下载中",
            Self::Done => "已完成",
            Self::Failed => "失败",
            Self::Cancelled => "已取消",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Queued => "queued",
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

/// 整批登记为排队。`jobs` 是（显示路径, 大小）。
pub(crate) fn start_batch(batch: u64, jobs: &[(String, u64)]) {
    with_records(|records| push_batch(records, batch, jobs));
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

/// 把所有进行中的记录标为已取消（同一时刻只有一个批次，故不需要批号）。
///
/// 用户点取消时 `reason` 为空（状态文字已说明）；线程起不来时用它附带原因。
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

fn push_batch(records: &mut Vec<Transfer>, batch: u64, jobs: &[(String, u64)]) {
    for (index, (name, size)) in jobs.iter().enumerate() {
        records.push(Transfer {
            batch,
            index,
            name: name.clone(),
            state: TransferState::Queued,
            bytes: 0,
            total: (*size > 0).then_some(*size),
            detail: String::new(),
        });
    }
    trim(records);
}

fn begin_in(records: &mut [Transfer], batch: u64, index: usize) {
    if let Some(record) = find(records, batch, index) {
        if record.state == TransferState::Queued {
            record.state = TransferState::Running;
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

fn percent_of(record: &Transfer) -> Option<i32> {
    match record.total {
        Some(total) if total > 0 => {
            Some(((record.bytes as f64 / total as f64) * 100.0).clamp(0.0, 100.0) as i32)
        }
        _ => None,
    }
}

fn text(value: &str) -> QVariant {
    QVariant::from(&QString::from(value))
}

fn insert(map: &mut QVariantMap, key: &str, value: QVariant) {
    map.insert_clone(&QString::from(key), &value);
}

/// 记录的稳定标识（QML 用它把实时进度贴到对应的行上）。
fn record_key(record: &Transfer) -> String {
    format!("{}:{}", record.batch, record.index)
}

fn record_map(record: &Transfer) -> QVariantMap {
    let percent = percent_of(record);
    let running = record.state == TransferState::Running;
    let progress = match record.state {
        TransferState::Done => 100,
        TransferState::Running => percent.unwrap_or(-1),
        TransferState::Failed | TransferState::Cancelled => percent.unwrap_or(0),
        TransferState::Queued => 0,
    };
    let progress_text = if running {
        percent
            .map(|value| format!("{value}%"))
            .unwrap_or_else(|| format_size(record.bytes))
    } else {
        String::new()
    };
    let size_text = match record.state {
        TransferState::Running => match record.total {
            Some(total) => format!("{} / {}", format_size(record.bytes), format_size(total)),
            None => format_size(record.bytes),
        },
        TransferState::Done => format_size(record.total.unwrap_or(record.bytes).max(record.bytes)),
        _ => record
            .total
            .map(format_size)
            .or_else(|| (record.bytes > 0).then(|| format_size(record.bytes)))
            .unwrap_or_else(|| "—".to_owned()),
    };

    let mut map = QVariantMap::default();
    insert(&mut map, "key", text(&record_key(record)));
    insert(&mut map, "name", text(&record.name));
    insert(&mut map, "state", text(record.state.name()));
    insert(&mut map, "stateText", text(record.state.text()));
    insert(&mut map, "running", QVariant::from(&running));
    insert(&mut map, "progress", QVariant::from(&progress));
    insert(&mut map, "progressText", text(&progress_text));
    insert(&mut map, "sizeText", text(&size_text));
    insert(&mut map, "detail", text(&record.detail));
    map
}

/// 一条记录 → 进度行的 map（只带下载中要刷新的字段）。
fn live_map(record: &Transfer) -> QVariantMap {
    let percent = percent_of(record);
    let progress_text = percent
        .map(|value| format!("{value}%"))
        .unwrap_or_else(|| format_size(record.bytes));
    let size_text = match record.total {
        Some(total) => format!("{} / {}", format_size(record.bytes), format_size(total)),
        None => format_size(record.bytes),
    };

    let mut map = QVariantMap::default();
    insert(&mut map, "key", text(&record_key(record)));
    insert(&mut map, "progress", QVariant::from(&percent.unwrap_or(-1)));
    insert(&mut map, "progressText", text(&progress_text));
    insert(&mut map, "sizeText", text(&size_text));
    map
}

/// 超过显示上限时追加的提示行（ListView 里最后一行）。
fn notice_map(hidden: usize) -> QVariantMap {
    let mut map = QVariantMap::default();
    insert(
        &mut map,
        "name",
        text(&format!(
            "还有 {hidden} 条记录未显示（页面只显示前 500 条）"
        )),
    );
    insert(&mut map, "state", text("notice"));
    insert(&mut map, "stateText", text(""));
    insert(&mut map, "running", QVariant::from(&false));
    insert(&mut map, "progress", QVariant::from(&0));
    insert(&mut map, "progressText", text(""));
    insert(&mut map, "sizeText", text(""));
    insert(&mut map, "detail", text(""));
    map
}

#[cfg(test)]
#[path = "transfers_tests.rs"]
mod tests;
