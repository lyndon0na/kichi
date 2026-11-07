//! 暂停 / 继续与自动续跑标记。

use std::collections::BTreeSet;

use kichi_core::EntryKind;

use crate::cxxqt_object::transfers_rate::{NoteState, SpeedMeter};

use super::*;

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

/// 正常退出前给仍在排队 / 准备 / 上传中的上传行打自动续传标记。
///
/// 暂停、失败、取消、已中断和已完成行一律不标；文件夹批次里只有当时确实
/// 在途的子文件会被恢复，历史失败 / 中断的兄弟文件仍等用户显式重试。
pub(crate) fn mark_auto_resume_uploads() {
    with_records(|records| mark_auto_resume_uploads_in(records));
}

/// 启动时清掉自动续跑标记：必须在真正启动前写盘，避免随后崩溃又续一轮。
pub(crate) fn clear_auto_resume() {
    with_records(|records| clear_auto_resume_in(records));
}

pub(crate) fn mark_auto_resume_in(records: &mut [Transfer]) {
    let auto_batches: BTreeSet<u64> = records
        .iter()
        .filter(|record| {
            !record.child
                && !record.job.is_upload()
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

pub(crate) fn mark_auto_resume_uploads_in(records: &mut [Transfer]) {
    let batches: BTreeSet<u64> = records
        .iter()
        .filter(|record| {
            !record.child
                && record.job.is_upload()
                && matches!(
                    record.state,
                    TransferState::Queued | TransferState::Preparing | TransferState::Running
                )
        })
        .map(|record| record.batch)
        .collect();
    for record in records.iter_mut().filter(|record| record.job.is_upload()) {
        record.auto_resume = batches.contains(&record.batch)
            && matches!(
                record.state,
                TransferState::Queued | TransferState::Preparing | TransferState::Running
            );
    }
}

pub(crate) fn clear_auto_resume_in(records: &mut [Transfer]) {
    for record in records {
        record.auto_resume = false;
    }
}

/// 自动续传文件夹时只重排当时在途的成员；返回是否真的改到记录。
pub(crate) fn resume_auto_upload_folder(batch: u64, indices: &[usize]) -> bool {
    with_records(|records| resume_auto_upload_folder_in(records, batch, indices))
}

pub(crate) fn resume_auto_upload_folder_in(
    records: &mut [Transfer],
    batch: u64,
    indices: &[usize],
) -> bool {
    let indices: BTreeSet<usize> = indices.iter().copied().collect();
    let mut changed = false;
    for record in records.iter_mut().filter(|record| {
        record.batch == batch
            && record.job.kind == EntryKind::File
            && record.job.is_upload()
            && record.state == TransferState::Interrupted
            && indices.contains(&record.index)
    }) {
        record.state = TransferState::Queued;
        record.bytes = 0;
        record.total = (record.job.size > 0).then_some(record.job.size);
        record.detail.clear();
        record.upload_progress = None;
        record.speed = SpeedMeter::default();
        record.note = NoteState::default();
        changed = true;
    }
    if !changed {
        return false;
    }
    rebuild_batch_stats(records, batch);
    if let Some(parent) = records
        .iter_mut()
        .find(|record| record.batch == batch && is_folder_ledger(record))
    {
        parent.state = TransferState::Queued;
        parent.detail.clear();
        parent.empty_folder = false;
    }
    true
}

pub(crate) fn mark_pausing_in(records: &mut [Transfer], batch: u64) {
    for record in records.iter_mut().filter(|record| record.batch == batch) {
        if matches!(
            record.state,
            TransferState::Queued | TransferState::Preparing | TransferState::Running
        ) {
            record.state = TransferState::Pausing;
            record.detail.clear();
        }
    }
    rebuild_batch_stats(records, batch);
}

pub(crate) fn mark_paused_in(records: &mut [Transfer], batch: u64) {
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
    rebuild_batch_stats(records, batch);
}

pub(crate) fn pause_queued_in(records: &mut [Transfer], batch: u64) {
    for record in records.iter_mut().filter(|record| record.batch == batch) {
        if matches!(
            record.state,
            TransferState::Queued | TransferState::Preparing
        ) {
            record.state = TransferState::Paused;
            record.detail.clear();
        }
    }
    rebuild_batch_stats(records, batch);
}

pub(crate) fn resume_paused_in(records: &mut [Transfer], batch: u64) {
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
    rebuild_batch_stats(records, batch);
}
