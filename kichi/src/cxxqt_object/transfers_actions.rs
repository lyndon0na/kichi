//! 传输记录的逐条动作：重试目标、删除旧行、取落盘路径。
//!
//! 只操作记录表；入队由 `downloads.rs` 调 `queue::reserve_retry`。重试的记录
//! 身份是 [`record_key`]（文件夹总账行 `batch:folder`，文件 / 子行 `batch:index`），
//! 与 QML 里行的 `key` 一致。

use std::path::PathBuf;

use kichi_core::EntryKind;

use super::transfers::{
    folder_download_path, has_done_child, is_folder_ledger, record_key, refresh_parent_state,
    with_records, Transfer, TransferJob, TransferState,
};

/// 一条失败记录的重试种子；不是 `Failed` 就是 `None`（唯一的可重试闸门）。
///
/// 文件夹总账行只有「读目录前整体失败」（还没有任何子行）才能整批重试；
/// 部分失败 / 全部失败都在详情页逐个重试失败文件。
pub(crate) fn retry_target(key: &str) -> Option<TransferJob> {
    with_records(|records| retry_target_in(records, key))
}

fn retry_target_in(records: &[Transfer], key: &str) -> Option<TransferJob> {
    let record = records
        .iter()
        .find(|record| record_key(record) == key && record.state == TransferState::Failed)?;
    if is_folder_ledger(record)
        && records
            .iter()
            .any(|row| row.batch == record.batch && row.child)
    {
        return None;
    }
    Some(record.job.clone())
}

/// 删掉一条旧记录（入队成功之后才调）。返回是否真的删了。
pub(crate) fn remove(key: &str) -> bool {
    with_records(|records| remove_in(records, key))
}

fn remove_in(records: &mut Vec<Transfer>, key: &str) -> bool {
    let Some(index) = records.iter().position(|record| record_key(record) == key) else {
        return false;
    };
    let removed = records.remove(index);
    if removed.child {
        refresh_parent_state(records, removed.batch);
        remove_empty_parent(records, removed.batch);
    }
    true
}

/// 最后一个子行被重试移走后，已结束的父行也一起消失（详情页自动返回）。
fn remove_empty_parent(records: &mut Vec<Transfer>, batch: u64) {
    if records
        .iter()
        .any(|record| record.batch == batch && record.child)
    {
        return;
    }
    let Some(index) = records
        .iter()
        .position(|record| record.batch == batch && is_folder_ledger(record))
    else {
        return;
    };
    if records[index].state.is_finished() {
        records.remove(index);
    }
}

/// 一条记录的落盘路径：完成行有文件路径；文件夹父行在至少一个子文件完成后
/// 给出文件夹路径，否则不可定位。
pub(crate) fn path_for(key: &str) -> Option<PathBuf> {
    with_records(|records| path_for_in(records, key))
}

fn path_for_in(records: &[Transfer], key: &str) -> Option<PathBuf> {
    let record = records.iter().find(|record| record_key(record) == key)?;
    if record.job.kind == EntryKind::Folder {
        if has_done_child(records, record.batch) {
            return folder_download_path(record);
        }
        return None;
    }
    record.path.clone()
}

#[cfg(test)]
#[path = "transfers_actions_tests.rs"]
mod tests;
