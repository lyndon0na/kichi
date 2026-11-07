//! 记录表 / 目录树的全局锁、批次号、会话号与淘汰。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use kichi_core::EntryKind;

use super::*;

/// 文件夹批次的目录树（`queue_collect::Collected.dirs`，相对下载根目录）。
///
/// 读目录那一刻记下，落盘时写在总账行上，读回后挂回这里。批次被清掉 / 淘汰 /
/// 删行后由 [`prune_trees`] 收走，不会留到进程退出。锁顺序：先 `records`、再 `trees`。
pub(crate) fn trees() -> &'static Mutex<HashMap<u64, Vec<Vec<String>>>> {
    static TREES: OnceLock<Mutex<HashMap<u64, Vec<Vec<String>>>>> = OnceLock::new();
    TREES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 文件夹目录树加锁（中毒时继续用内部数据）；`transfers_history` 写盘时也走这里。
pub(crate) fn with_trees<T>(f: impl FnOnce(&mut HashMap<u64, Vec<Vec<String>>>) -> T) -> T {
    let mut guard = trees().lock().unwrap_or_else(|e| e.into_inner());
    f(&mut guard)
}

/// 读回后把落盘的目录树挂回内存（只留记录表里还在的批次）。
///
/// 写盘时要从这里取树，不挂回来的话重启后的第一次写盘就把它们丢了。
/// 调用方持有 `records` 锁（锁顺序：先 records、再 trees）。
pub(crate) fn restore_trees(trees: Vec<(u64, Vec<Vec<String>>)>, records: &[Transfer]) {
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
pub(crate) fn prune_trees() {
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

pub(crate) fn records() -> &'static Mutex<Vec<Transfer>> {
    static RECORDS: OnceLock<Mutex<Vec<Transfer>>> = OnceLock::new();
    RECORDS.get_or_init(|| Mutex::new(Vec::new()))
}

/// 记录表加锁（中毒时继续用内部数据）；`transfers_actions` 的逐条动作也走这里。
pub(crate) fn with_records<T>(f: impl FnOnce(&mut Vec<Transfer>) -> T) -> T {
    let mut guard = records().lock().unwrap_or_else(|e| e.into_inner());
    f(&mut guard)
}

/// 取一个新的批次序号。
///
/// 启动时由 [`raise_batch_counter`] 跳到历史最大值之后，避免与读回的行撞 key。
pub(crate) fn new_batch() -> u64 {
    counter().fetch_add(1, Ordering::SeqCst)
}

pub(crate) fn counter() -> &'static AtomicU64 {
    static NEXT: OnceLock<AtomicU64> = OnceLock::new();
    NEXT.get_or_init(|| AtomicU64::new(1))
}

/// 批次计数器至少走到 `max_batch` 之后（读回历史时调用）。
pub(crate) fn raise_batch_counter(max_batch: u64) {
    let next = max_batch.saturating_add(1);
    counter().fetch_max(next, Ordering::SeqCst);
}

/// 这一轮运行的会话号（读回后的新行都盖上它，用来区分「上一轮没跑完」）。
pub(crate) fn session_slot() -> &'static AtomicU64 {
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

/// 「清空已完成记录」：清掉整张记录表里所有整批都是 `Done` 的记录。
///
/// 失败 / 已中断的批次是待重试项，暂停与进行中的批次更不能碰。
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
        crate::cxxqt_object::transfers_detail::clear_selection();
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

/// 正在扫描 / 上传的顶层上传批次数（扫描中的父行也算进行中）。
pub(crate) fn upload_active_count() -> i32 {
    with_records(|records| {
        let count = records
            .iter()
            .filter(|record| !record.child && record.job.is_upload() && record.state.is_active())
            .count();
        i32::try_from(count).unwrap_or(i32::MAX)
    })
}

/// 整批都已完成（`Done`）的批次号（升序）。
///
/// 失败与已中断的批次是「还要重试的东西」，不参与「清空已完成」。
pub(crate) fn finished_batches(records: &[Transfer]) -> Vec<u64> {
    let mut batches: Vec<u64> = records.iter().map(|record| record.batch).collect();
    batches.sort_unstable();
    batches.dedup();
    batches.retain(|batch| {
        records
            .iter()
            .filter(|record| record.batch == *batch)
            .all(|record| record.state == TransferState::Done)
    });
    batches
}

/// 可以按内存上限淘汰的整批：已结束，或整批只剩「已中断」。
///
/// 只用于 [`trim`]（进程内 2000 条的内存闸门）；「清空已完成记录」不碰已中断的行。
pub(crate) fn evictable_batches(records: &[Transfer]) -> Vec<u64> {
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

/// 清空已完成的**整批**：正在下载 / 排队 / 暂停 / 失败 / 已中断的批次完整保留。
pub(crate) fn clear_finished_in(records: &mut Vec<Transfer>) {
    let removable = finished_batches(records);
    if removable.is_empty() {
        return;
    }
    records.retain(|record| !removable.contains(&record.batch));
}

/// 「清空已完成记录」：只清全部行都 `Done` 的整批。
///
/// 与 `clear_finished_in` 同一口径；分出来只是为了让调用点说清口径。
pub(crate) fn clear_all_in(records: &mut Vec<Transfer>) {
    clear_finished_in(records);
}

/// 进程内最多保留的记录数（超出丢最早的已结束记录，进行中的绝不丢）。
///
/// 落盘的历史不受这个数限制（ADR 0014：不设条数上限）。
pub(crate) const MAX_RECORDS: usize = 2000;

/// 超过上限时按整批淘汰最早的已结束批次；进行中的批次绝不删。
///
/// 只剩最新一个可淘汰批次时保留它：一个超大文件夹刚下完，
/// 不能因为记录数超限就整条从界面上消失。
pub(crate) fn trim(records: &mut Vec<Transfer>) {
    while records.len() > MAX_RECORDS {
        let evictable = evictable_batches(records);
        if evictable.len() < 2 {
            break;
        }
        let oldest = evictable[0];
        records.retain(|record| record.batch != oldest);
    }
}
