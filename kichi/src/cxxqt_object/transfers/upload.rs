//! 上传文件夹批次的目录树、子文件状态与原地重试。

use std::time::Instant;

use kichi_core::{EntryKind, UploadStage};

use crate::cxxqt_object::transfers_rate::{NoteState, SpeedMeter};

use super::*;

/// 上传文件夹轮到了：父行进入「上传中」，worker 开始建目录 / 传文件。
pub(crate) fn mark_upload_folder_running(batch: u64) {
    with_records(|records| {
        if let Some(parent) = records
            .iter_mut()
            .find(|record| record.batch == batch && is_folder_ledger(record))
        {
            if matches!(
                parent.state,
                TransferState::Queued | TransferState::Preparing
            ) {
                parent.state = TransferState::Running;
            }
        }
    });
}

/// 上传文件夹父行的重试种子（扫描完成回调要用它构造子文件）。
pub(crate) fn upload_folder_job(batch: u64) -> Option<TransferJob> {
    with_records(|records| {
        records
            .iter()
            .find(|record| record.batch == batch && is_folder_ledger(record))
            .map(|record| record.job.clone())
    })
}

/// 暂停继续前刷新调度器快照：目录 id 会随 worker 进度增长，不能一直用入队时的空表。
pub(crate) fn upload_folder_resume_state(batch: u64) -> Option<(TransferJob, Vec<UploadDir>)> {
    with_records(|records| {
        records
            .iter()
            .find(|record| record.batch == batch && is_folder_ledger(record))
            .map(|record| (record.job.clone(), record.upload_dirs.clone()))
    })
}

/// 上传文件夹父行当前状态；扫描回调据此丢弃已经取消 / 中断的批次。
pub(crate) fn upload_folder_state(batch: u64) -> Option<TransferState> {
    with_records(|records| {
        records
            .iter()
            .find(|record| record.batch == batch && is_folder_ledger(record))
            .map(|record| record.state)
    })
}

/// 扫描 / 建云端目录期间给上传父行的说明。
pub(crate) fn set_upload_folder_detail(batch: u64, detail: &str) {
    with_records(|records| {
        if let Some(parent) = records
            .iter_mut()
            .find(|record| record.batch == batch && is_folder_ledger(record))
        {
            parent.detail = detail.to_owned();
        }
    });
}

/// 某个云端目录后来创建成功：移除它自己的失败标记（父行可能因此恢复）。
pub(crate) fn clear_upload_dir_failure(batch: u64, path: &[String]) {
    with_records(|records| {
        let mut changed = false;
        if let Some(parent) = records
            .iter_mut()
            .find(|record| record.batch == batch && is_folder_ledger(record))
        {
            let before = parent.upload_failed_dirs.len();
            parent
                .upload_failed_dirs
                .retain(|failed| failed.as_slice() != path);
            changed = parent.upload_failed_dirs.len() != before;
        }
        if changed {
            refresh_parent_state(records, batch);
        }
    });
}

/// 上传文件夹父行记录已创建的云端目录；同时把根目录 id 同步给所有子文件，
/// 这样失败子文件重试时不必重新猜根目录。
pub(crate) fn set_upload_dirs(batch: u64, dirs: &[UploadDir]) {
    with_records(|records| {
        let root_id = dirs
            .iter()
            .find(|dir| dir.path.len() == 1)
            .map(|dir| dir.id.clone())
            .unwrap_or_default();
        for record in records.iter_mut().filter(|record| record.batch == batch) {
            if is_folder_ledger(record) {
                record.upload_dirs = dirs.to_vec();
            }
            if let Some(upload) = record.job.upload.as_mut() {
                if !root_id.is_empty() {
                    upload.batch_root_id = root_id.clone();
                }
            }
        }
    });
}

/// 子文件的目录链已就绪：写入直接云端父目录 id，供历史重试直接复用。
pub(crate) fn set_upload_file_parent(batch: u64, index: usize, parent_id: &str) {
    with_records(|records| {
        if let Some(record) = find_file(records, batch, index) {
            if let Some(upload) = record.job.upload.as_mut() {
                upload.cloud_parent_id = parent_id.to_owned();
            }
        }
    });
}

/// 一个云端目录创建失败：只把它子树里的待上传文件标失败，父行记一个跳过项。
pub(crate) fn fail_upload_subtree(batch: u64, path: &[String], reason: &str) {
    with_records(|records| {
        for record in records.iter_mut().filter(|record| {
            record.batch == batch
                && record.child
                && record.job.direction == TransferDirection::Upload
                && record.job.sub_dir.starts_with(path)
                && matches!(
                    record.state,
                    TransferState::Queued | TransferState::Preparing | TransferState::Running
                )
        }) {
            record.state = TransferState::Failed;
            record.detail = reason.to_owned();
            record.speed = SpeedMeter::default();
            record.note = NoteState::default();
        }
        rebuild_batch_stats(records, batch);
        if let Some(parent) = records
            .iter_mut()
            .find(|record| record.batch == batch && is_folder_ledger(record))
        {
            if !parent
                .upload_failed_dirs
                .iter()
                .any(|failed| failed.as_slice() == path)
            {
                parent.upload_failed_dirs.push(path.to_vec());
            }
            // detail 在失败前通常是「正在创建云端目录：…」；失败后必须换成
            // 真实原因，否则空目录批次会一直显示进度文案而不是错误。
            parent.detail = reason.to_owned();
        }
        refresh_parent_state(records, batch);
    });
}

/// 上传文件夹批次收尾：没有子行时由空目录逻辑落状态，有子行时按子行和
/// 目录失败集合重新聚合。
pub(crate) fn finish_upload_folder(batch: u64) {
    with_records(|records| {
        let has_children = records
            .iter()
            .any(|record| record.batch == batch && record.child);
        if !has_children {
            if let Some(parent) = records
                .iter_mut()
                .find(|record| record.batch == batch && is_folder_ledger(record))
            {
                if !matches!(
                    parent.state,
                    TransferState::Cancelled | TransferState::Interrupted
                ) {
                    parent.empty_folder = parent.upload_failed_dirs.is_empty();
                    parent.state = if parent.empty_folder {
                        parent.detail.clear();
                        TransferState::Done
                    } else {
                        TransferState::Failed
                    };
                }
            }
            return;
        }
        refresh_parent_state(records, batch);
    });
}

/// 把上传文件夹父行重置为「准备中」，用于失败后重新扫描本地目录。
pub(crate) fn reset_upload_folder_for_scan(batch: u64) {
    with_records(|records| {
        for record in records.iter_mut().filter(|record| record.batch == batch) {
            record.detail.clear();
            record.speed = SpeedMeter::default();
            record.note = NoteState::default();
            record.upload_progress = None;
            record.upload_dirs.clear();
            record.upload_failed_dirs.clear();
            if let Some(upload) = record.job.upload.as_mut() {
                upload.batch_root_id.clear();
                upload.cloud_parent_id.clear();
            }
        }
        if let Some(parent) = records
            .iter_mut()
            .find(|record| record.batch == batch && is_folder_ledger(record))
        {
            parent.state = TransferState::Preparing;
            parent.skipped = 0;
            parent.empty_folder = false;
        }
    });
}

/// 失败 / 已中断的上传子文件重新排队，不新建批次、不重传已完成兄弟文件。
pub(crate) fn retry_upload_child(batch: u64, index: usize) -> bool {
    with_records(|records| {
        let changed = update_child_record(records, batch, index, true, |child| {
            if !child.job.is_upload()
                || !matches!(
                    child.state,
                    TransferState::Failed | TransferState::Interrupted
                )
            {
                return false;
            }
            child.state = TransferState::Queued;
            child.bytes = 0;
            child.total = (child.job.size > 0).then_some(child.job.size);
            child.detail.clear();
            child.upload_progress = None;
            child.speed = SpeedMeter::default();
            child.note = NoteState::default();
            true
        });
        if !changed {
            return false;
        }
        if let Some(parent) = records
            .iter_mut()
            .find(|record| record.batch == batch && is_folder_ledger(record))
        {
            parent.state = TransferState::Queued;
            parent.detail.clear();
            parent.empty_folder = false;
        }
        true
    })
}

/// 上传文件夹父行的原地重试：保留已创建的云端目录和已完成文件，只把失败 /
/// 中断的成员重新置为排队。worker 会根据目录树补建缺失目录，因此失败的空目录
/// 也有重试入口，而不会重传已成功的兄弟文件。
pub(crate) fn retry_upload_folder(batch: u64) -> bool {
    with_records(|records| {
        let Some(parent) = records
            .iter()
            .find(|record| record.batch == batch && is_folder_ledger(record))
        else {
            return false;
        };
        if !parent.job.is_upload()
            || !matches!(
                parent.state,
                TransferState::Failed | TransferState::Interrupted
            )
        {
            return false;
        }

        for record in records.iter_mut().filter(|record| {
            record.batch == batch
                && record.child
                && record.job.is_upload()
                && matches!(
                    record.state,
                    TransferState::Failed
                        | TransferState::Interrupted
                        | TransferState::Queued
                        | TransferState::Preparing
                )
        }) {
            record.state = TransferState::Queued;
            record.bytes = 0;
            record.total = (record.job.size > 0).then_some(record.job.size);
            record.detail.clear();
            record.upload_progress = None;
            record.speed = SpeedMeter::default();
            record.note = NoteState::default();
        }
        rebuild_batch_stats(records, batch);

        if let Some(parent) = records
            .iter_mut()
            .find(|record| record.batch == batch && is_folder_ledger(record))
        {
            parent.state = TransferState::Queued;
            parent.detail.clear();
            parent.empty_folder = false;
            // 旧失败集合只代表上一轮；worker 会按本轮实际结果重新收集。
            parent.upload_failed_dirs.clear();
        }
        true
    })
}

pub(crate) fn upload_progress_in(
    records: &mut [Transfer],
    batch: u64,
    index: usize,
    progress: UploadProgressState,
) {
    update_child_record(records, batch, index, false, |record| {
        if record.state != TransferState::Running || !record.job.is_upload() {
            return false;
        }
        let before = (record.bytes, record.total, record.upload_progress);
        let old_stage = record.upload_progress.map(|old| old.stage);
        if progress.stage == UploadStage::Uploading && old_stage != Some(UploadStage::Uploading) {
            record.speed.begin(Instant::now());
        }
        record.upload_progress = Some(progress);
        // 断点进度只接受“已完成分片”口径；Uploading / Finalizing 才带这个值。
        if let Some(completed) = progress.completed_bytes {
            record.bytes = completed;
        }
        record.total = (record.job.size > 0).then_some(record.job.size);
        if progress.stage == UploadStage::Uploading {
            record.speed.observe(Instant::now(), progress.bytes);
        }
        before != (record.bytes, record.total, record.upload_progress)
    });
}

/// 上传子文件当前状态（worker 在开始前用它跳过已被目录失败标记的行）。
pub(crate) fn upload_child_state(batch: u64, index: usize) -> Option<TransferState> {
    with_records(|records| {
        records
            .iter()
            .find(|record| {
                record.job.kind == EntryKind::File && record.batch == batch && record.index == index
            })
            .map(|record| record.state)
    })
}

/// 上传成功：记录服务端最终名字，不写本地路径（因此不提供「打开位置」）。
pub(crate) fn finish_upload_ok(batch: u64, index: usize, result: &kichi_core::UploadResult) {
    with_records(|records| {
        update_child_record(records, batch, index, false, |record| {
            record.state = TransferState::Done;
            record.bytes = record.job.size;
            record.total = Some(record.job.size);
            record.detail.clear();
            record.path = None;
            record.job.name = result.name.clone();
            record.upload_progress = Some(UploadProgressState {
                stage: UploadStage::Finalizing,
                bytes: record.job.size,
                completed_bytes: Some(record.job.size),
                total: record.job.size,
            });
            true
        });
    });
}
