//! 传输记录的逐条动作：重试目标、删除旧行、取落盘路径。
//!
//! 只操作记录表；入队由 `downloads.rs` 调 `queue::reserve_retry`。重试的记录
//! 身份是 [`record_key`]（`batch:index`），与 QML 里行的 `key` 一致。

use std::path::PathBuf;

use super::transfers::{record_key, with_records, Transfer, TransferJob, TransferState};

/// 一条失败记录的重试种子；不是 `Failed` 就是 `None`（唯一的可重试闸门）。
pub(crate) fn retry_target(key: &str) -> Option<TransferJob> {
    with_records(|records| retry_target_in(records, key))
}

fn retry_target_in(records: &[Transfer], key: &str) -> Option<TransferJob> {
    records
        .iter()
        .find(|record| record_key(record) == key && record.state == TransferState::Failed)
        .map(|record| record.job.clone())
}

/// 删掉一条旧记录（入队成功之后才调）。返回是否真的删了。
pub(crate) fn remove(key: &str) -> bool {
    with_records(|records| remove_in(records, key))
}

fn remove_in(records: &mut Vec<Transfer>, key: &str) -> bool {
    match records.iter().position(|record| record_key(record) == key) {
        Some(index) => {
            records.remove(index);
            true
        }
        None => false,
    }
}

/// 一条记录的落盘路径（只有完成行有）。
pub(crate) fn path_for(key: &str) -> Option<PathBuf> {
    with_records(|records| path_for_in(records, key))
}

fn path_for_in(records: &[Transfer], key: &str) -> Option<PathBuf> {
    records
        .iter()
        .find(|record| record_key(record) == key)
        .and_then(|record| record.path.clone())
}

#[cfg(test)]
#[path = "transfers_actions_tests.rs"]
mod tests;
