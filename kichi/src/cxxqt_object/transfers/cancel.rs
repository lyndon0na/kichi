//! 取消、断点清理、登出中断与记录淘汰前的收尾。

use std::collections::BTreeSet;
use std::path::PathBuf;

use kichi_core::EntryKind;

use crate::cxxqt_object::transfers_rate::{NoteState, SpeedMeter};

use super::*;

/// 只取消一个方向的活动记录（下载页的「取消全部」不能顺手取消上传）。
pub(crate) fn cancel_active_for(direction: TransferDirection, reason: &str) {
    with_records(|records| cancel_active_for_in(records, direction, reason));
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
        rebuild_batch_stats(records, batch);
        refresh_parent_state(records, batch);
    });
    discard_batch_parts(batch);
}

/// 取消即删记录：整批取消的批次连子行一起删，文件夹里单个子文件取消只删该行。
///
/// 取消时下载断点已经删掉、上传也丢弃了断点，记录留着没有可恢复的东西。
/// 还在调度器里的批次先不动：下载线程退出后还要按记录清掉剩余断点
/// （`discard_batch_parts`），等它释放调度槽后的下一次同步再删。
///
/// 由 `sync_transfers()` 在重建模型前调用；返回是否删掉了行。
pub(crate) fn prune_cancelled() -> bool {
    // 先只读候选：不能在持有记录锁时再去拿调度锁（`queue::is_pending` 是反方向）。
    let (cancelled_batches, has_cancelled_child, child_parts) = with_records(|records| {
        let batches: BTreeSet<u64> = records
            .iter()
            .filter(|record| !record.child && record.state == TransferState::Cancelled)
            .map(|record| record.batch)
            .collect();
        // 子文件单独取消且父行没取消：记录删掉后 `discard_batch_parts` 就找不到
        // 它了，这里先把断点路径记下来，删完记录立刻清掉。
        let has_child = records
            .iter()
            .any(|record| record.child && record.state == TransferState::Cancelled);
        let parts: Vec<(String, PathBuf)> = records
            .iter()
            .filter(|record| {
                record.child
                    && record.state == TransferState::Cancelled
                    && !batches.contains(&record.batch)
                    && !record.job.is_upload()
            })
            .map(|record| (record.job.id.clone(), record.job.dir.clone()))
            .collect();
        (batches, has_child, parts)
    });
    if cancelled_batches.is_empty() && !has_cancelled_child {
        return false;
    }
    let busy: BTreeSet<u64> = cancelled_batches
        .iter()
        .copied()
        .filter(|batch| {
            crate::cxxqt_object::queue::is_batch_busy(*batch)
                || crate::cxxqt_object::upload_queue::is_batch_busy(*batch)
        })
        .collect();
    let removed = with_records(|records| prune_cancelled_in(records, &busy));
    if removed {
        prune_trees();
        for (id, dir) in child_parts {
            crate::session::partials::discard(&id, &dir);
        }
    }
    removed
}

pub(crate) fn prune_cancelled_in(records: &mut Vec<Transfer>, busy: &BTreeSet<u64>) -> bool {
    let cancelled_batches: BTreeSet<u64> = records
        .iter()
        .filter(|record| !record.child && record.state == TransferState::Cancelled)
        .map(|record| record.batch)
        .collect();
    let removable: BTreeSet<u64> = cancelled_batches
        .iter()
        .copied()
        .filter(|batch| !busy.contains(batch))
        .collect();
    let before = records.len();
    let mut touched: BTreeSet<u64> = BTreeSet::new();
    records.retain(|record| {
        if removable.contains(&record.batch) {
            return false;
        }
        // 子文件单独取消（父行没取消）：只删这一行，父行按剩余子文件重新聚合。
        // 整批取消但线程还在收尾时要保留全部行，等 `discard_batch_parts` 清完。
        if record.child
            && record.state == TransferState::Cancelled
            && !cancelled_batches.contains(&record.batch)
        {
            touched.insert(record.batch);
            return false;
        }
        true
    });
    if records.len() == before {
        return false;
    }
    rebuild_all_batch_stats(records);
    for batch in touched {
        refresh_parent_state(records, batch);
    }
    true
}

/// 删掉某一批里所有未完成文件行的断点；已完成文件没有 part，不受影响。
pub(crate) fn discard_batch_parts(batch: u64) {
    discard_parts(batch, None);
}

/// 只清一个文件夹子文件在暂停 / 排队取消后的断点。
pub(crate) fn discard_file_part(batch: u64, index: usize) {
    discard_parts(batch, Some(index));
}

/// 取消上传时要摘掉的 KWallet 断点（本地路径 + 当时的直接父目录 id）。
///
/// 文件夹子文件的直接父目录可能是建出来的子目录；同一文件的旧记录还可能
/// 保存在重命名前的目录 id 下，因此把记录里的目标目录、最近一次上传父目录
/// 和目录树中的对应 id 都列出来，逐个查找并删除。
///
/// `Running` / `Pausing` 仍由正在收尾的 worker 删除断点并清理；主线程只处理
/// 还没有线程接手的行，避免把刚好成功、已经写进云端的上传误删。
pub(crate) fn upload_resume_targets(batch: u64, index: Option<usize>) -> Vec<(PathBuf, String)> {
    with_records(|records| upload_resume_targets_in(records, batch, index))
}

pub(crate) fn upload_resume_targets_in(
    records: &[Transfer],
    batch: u64,
    index: Option<usize>,
) -> Vec<(PathBuf, String)> {
    let mut targets = Vec::new();
    for record in records.iter().filter(|record| {
        record.batch == batch
            && record.job.kind == EntryKind::File
            && record.job.is_upload()
            && !matches!(
                record.state,
                TransferState::Running | TransferState::Pausing
            )
            && index.is_none_or(|index| record.index == index)
    }) {
        collect_upload_resume_targets(records, record, &mut targets);
    }
    targets
}

/// 「取消全部」只动仍可取消的上传行，失败 / 已中断的断点留下供重试。
pub(crate) fn cancellable_upload_resume_targets() -> Vec<(PathBuf, String)> {
    with_records(|records| {
        let mut targets = Vec::new();
        for record in records.iter().filter(|record| {
            record.job.kind == EntryKind::File
                && record.job.is_upload()
                && record.state.is_cancellable()
                && !matches!(
                    record.state,
                    TransferState::Running | TransferState::Pausing
                )
        }) {
            collect_upload_resume_targets(records, record, &mut targets);
        }
        targets
    })
}

pub(crate) fn collect_upload_resume_targets(
    records: &[Transfer],
    record: &Transfer,
    targets: &mut Vec<(PathBuf, String)>,
) {
    let Some(upload) = record.job.upload.as_ref() else {
        return;
    };
    let mut parent_ids = vec![upload.parent_id.as_str()];
    if !upload.cloud_parent_id.is_empty() {
        parent_ids.push(upload.cloud_parent_id.as_str());
    }
    if !record.job.sub_dir.is_empty() {
        if let Some(parent) = records.iter().find(|candidate| {
            candidate.batch == record.batch
                && is_folder_ledger(candidate)
                && candidate.job.is_upload()
        }) {
            if let Some(dir) = parent
                .upload_dirs
                .iter()
                .find(|dir| dir.path == record.job.sub_dir)
            {
                parent_ids.push(dir.id.as_str());
            }
        }
    }
    for parent_id in parent_ids {
        let parent_id = parent_id.trim();
        if targets
            .iter()
            .any(|(path, id)| *path == upload.local_path && id == parent_id)
        {
            continue;
        }
        targets.push((upload.local_path.clone(), parent_id.to_owned()));
    }
}

pub(crate) fn discard_parts(batch: u64, index: Option<usize>) {
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

pub(crate) fn cancel_file_in(records: &mut [Transfer], batch: u64, index: usize) {
    update_child_record(records, batch, index, false, |record| {
        if record.state.is_cancellable() {
            record.state = TransferState::Cancelled;
            record.detail.clear();
            true
        } else {
            false
        }
    });
}

pub(crate) fn interrupt_active_for(direction: TransferDirection) {
    with_records(|records| {
        for record in records
            .iter_mut()
            .filter(|record| record.job.direction == direction)
        {
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
        rebuild_all_batch_stats(records);
    });
}

/// 只把某一批里仍在途的上传行收成「已中断」。
///
/// 用于 429 / 配额 / 鉴权停队后各批次自己的收尾：不能全局标记，否则另一个
/// 批次刚成功、结果还没回写时会被误吞。
pub(crate) fn interrupt_active_batch(batch: u64) {
    with_records(|records| {
        for record in records.iter_mut().filter(|record| record.batch == batch) {
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
        rebuild_batch_stats(records, batch);
    });
}

/// 登出 / 退出时把指定方向的暂停行也收成已中断。
///
/// 下载暂停会保留断点，重登后仍可继续；上传暂停只存在于当前进程，调度器里
/// 的暂停项已被清掉，必须转成「已中断」才能给出重新入队的入口。
pub(crate) fn interrupt_paused_for(direction: TransferDirection) {
    with_records(|records| {
        for record in records
            .iter_mut()
            .filter(|record| record.job.direction == direction)
        {
            if record.state == TransferState::Paused {
                record.state = TransferState::Interrupted;
                record.detail.clear();
            }
        }
        rebuild_all_batch_stats(records);
    });
}
