//! 传输页文件夹详情：进程内选中的批次 + 子文件快照。
//!
//! 选中状态不落盘、不跨重启；记录被清空 / 按上限淘汰 / 子行全部
//! 被重试时，[`detail_snapshot_in`] 会自动清掉选中，详情页下一次刷新就关闭。
//!
//! 锁顺序固定为「先 records，再 selection」：`select_folder` / `detail_snapshot`
//! 与 `transfers::reset` 都遵守这个顺序，不会交叉加锁。
//!
//! 登出不清记录（`docs/adr/0014`），只清这里的选中；旧记录仍是可浏览 / 可重试的历史。

use std::sync::{Mutex, OnceLock};

use super::transfers::{is_folder_ledger, record_key, with_records, Transfer};
use super::transfers_view::build_detail_snapshot;
use super::QVariantList;

fn selection() -> &'static Mutex<Option<u64>> {
    static SELECTION: OnceLock<Mutex<Option<u64>>> = OnceLock::new();
    SELECTION.get_or_init(|| Mutex::new(None))
}

fn with_selection<T>(f: impl FnOnce(&mut Option<u64>) -> T) -> T {
    let mut guard = selection().lock().unwrap_or_else(|e| e.into_inner());
    f(&mut guard)
}

/// 选中一个文件夹批次：key 必须对应「已有子行」的文件夹总账行。
pub(crate) fn select_folder(key: &str) -> bool {
    with_records(|records| with_selection(|selected| select_folder_in(selected, records, key)))
}

pub(crate) fn select_folder_in(
    selected: &mut Option<u64>,
    records: &[Transfer],
    key: &str,
) -> bool {
    let Some(record) = records
        .iter()
        .find(|record| is_folder_ledger(record) && record_key(record) == key)
    else {
        return false;
    };
    if !records
        .iter()
        .any(|row| row.batch == record.batch && row.child)
    {
        return false;
    }
    *selected = Some(record.batch);
    true
}

/// 清掉选中的文件夹；返回之前是否有选中。
pub(crate) fn clear_selection() -> bool {
    with_selection(clear_selection_in)
}

pub(crate) fn clear_selection_in(selected: &mut Option<u64>) -> bool {
    selected.take().is_some()
}

/// 当前选中文件夹的子文件快照 + 详情页是否仍然有效。
pub(crate) fn detail_snapshot() -> (QVariantList, bool) {
    with_records(|records| with_selection(|selected| detail_snapshot_in(selected, records)))
}

/// 选中的批次不存在 / 不再是文件夹 / 已经没有子行时，自动清掉选中。
pub(crate) fn detail_snapshot_in(
    selected: &mut Option<u64>,
    records: &[Transfer],
) -> (QVariantList, bool) {
    let Some(batch) = *selected else {
        return (QVariantList::default(), false);
    };
    let exists = records
        .iter()
        .any(|record| record.batch == batch && is_folder_ledger(record));
    let has_children = records
        .iter()
        .any(|record| record.batch == batch && record.child);
    if !exists || !has_children {
        *selected = None;
        return (QVariantList::default(), false);
    }
    (build_detail_snapshot(records, batch), true)
}

#[cfg(test)]
#[path = "transfers_detail_tests.rs"]
mod tests;
