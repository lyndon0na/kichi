//! 传输记录的逐条动作：重试目标、删除旧行、取落盘路径。
//!
//! 只操作记录表；入队由 `downloads.rs` 调 `queue::reserve_retry`。重试的记录
//! 身份是 [`record_key`]（文件夹总账行 `batch:folder`，文件 / 子行 `batch:index`），
//! 与 QML 里行的 `key` 一致。
//!
//! 可重试的状态是 `Failed` 与 `Interrupted`（上次退出时没跑完）：后者按原 id、
//! 原下载目录、原子目录重新入队，`partials.json` 里还有断点就接着下
//! （`docs/adr/0014-download-history-on-disk.md`）。

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use kichi_core::EntryKind;

use super::transfers::{
    current_session, folder_download_path, has_done_child, is_folder_ledger, record_key,
    refresh_parent_state, row_from, with_records, with_trees, Transfer, TransferJob, TransferState,
};

/// 一条失败 / 已中断记录的重试种子；其余状态返回 `None`（唯一的可重试闸门）。
///
/// 文件夹总账行只有「读目录前整体失败或整个中断」（还没有任何子行）才能整批重试；
/// 部分失败 / 全部失败都在详情页逐个重试失败文件。
pub(crate) fn retry_target(key: &str) -> Option<TransferJob> {
    with_records(|records| retry_target_in(records, key))
}

fn retry_target_in(records: &[Transfer], key: &str) -> Option<TransferJob> {
    let record = records.iter().find(|record| {
        record_key(record) == key
            && matches!(
                record.state,
                TransferState::Failed | TransferState::Interrupted
            )
    })?;
    if is_folder_ledger(record)
        && records
            .iter()
            .any(|row| row.batch == record.batch && row.child)
    {
        return None;
    }
    Some(record.job.clone())
}

/// 这一行是「读目录前整体失败 / 整个中断」的文件夹总账行吗（整批重试用自己的目录树重来）。
pub(crate) fn is_whole_folder(key: &str) -> bool {
    with_records(|records| super::transfers_view::is_whole_folder_ledger(records, key))
}

/// 把整个文件夹批次抽出来准备整批重试：删掉旧行，返回新批次号与重试种子。
///
/// 调用前已经用 [`retry_target`] 确认过是「读目录前整体失败 / 中断」的文件夹；
/// 目录树不在（没有 `dirs` 的旧文件）也照样能重试——重试会重新读一遍目录。
/// 登记失败时用 [`restore_folder`] 把旧行放回去。
pub(crate) fn take_whole_folder(key: &str) -> Option<(u64, TransferJob)> {
    let found = with_records(|records| {
        let record = records
            .iter()
            .find(|record| record_key(record) == key && is_folder_ledger(record))?;
        let taken = (record.batch, record.created_unix, record.job.clone());
        records.retain(|row| row.batch != taken.0);
        Some(taken)
    });
    let (batch, created, job) = found?;
    let dirs = with_trees(|trees| trees.remove(&batch)).unwrap_or_default();
    stash()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(batch, (dirs, created));
    Some((batch, job))
}

/// 登记失败时把抽走的整批放回去：总账行回来、回到原来那天。
pub(crate) fn restore_folder(batch: u64, job: &TransferJob) {
    let (dirs, created) =
        take_stash(batch).unwrap_or_else(|| (Vec::new(), super::transfers_history::now_unix()));
    let session = current_session();
    with_records(|records| {
        if records.iter().any(|record| record.batch == batch) {
            return;
        }
        // 放回去的是「已中断」而不是「排队中」：它没在调度器里，写成排队会永远不动
        let mut row = row_from(batch, 0, &job.id, job, created, session);
        row.state = TransferState::Interrupted;
        records.push(row);
        records.sort_by_key(|record| (record.batch, record.index));
    });
    if !dirs.is_empty() {
        with_trees(|trees| {
            trees.insert(batch, dirs);
        });
    }
}

/// 整批重试登记成功：丢掉暂存的目录树（新批次会重新读目录）。
pub(crate) fn forget_stash(batch: u64) {
    let _ = take_stash(batch);
}

/// 「抽出来等重试」的文件夹目录树与入队时刻（不在记录表里，单独一小份）。
type Stash = (Vec<Vec<String>>, i64);

fn stash() -> &'static Mutex<std::collections::HashMap<u64, Stash>> {
    static STASH: OnceLock<Mutex<std::collections::HashMap<u64, Stash>>> = OnceLock::new();
    STASH.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

fn take_stash(batch: u64) -> Option<Stash> {
    let mut guard = stash()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.remove(&batch)
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

/// 最后一个子行被重试移走后，已结束 / 已中断的父行也一起消失（详情页自动返回）。
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
    if records[index].state.is_finished() || records[index].state == TransferState::Interrupted {
        records.remove(index);
    }
}

/// 一条记录的落盘路径：完成行有文件路径；文件夹父行在至少一个子文件完成后
/// 给出文件夹路径，空文件夹（Done 且没有子行）给建出来的目录。
pub(crate) fn path_for(key: &str) -> Option<PathBuf> {
    with_records(|records| path_for_in(records, key))
}

fn path_for_in(records: &[Transfer], key: &str) -> Option<PathBuf> {
    let record = records.iter().find(|record| record_key(record) == key)?;
    if record.job.kind == EntryKind::Folder {
        if let Some(path) = &record.path {
            return Some(path.clone());
        }
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
