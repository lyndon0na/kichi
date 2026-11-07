//! 文件夹批次的子行聚合与父行状态重算。

use std::collections::{BTreeMap, BTreeSet, HashMap};

use kichi_core::UploadStage;

use super::*;

/// 一个文件夹批次的子行聚合。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct BatchStats {
    pub(crate) total: usize,
    pub(crate) queued: usize,
    pub(crate) preparing: usize,
    pub(crate) running: usize,
    pub(crate) pausing: usize,
    pub(crate) paused: usize,
    pub(crate) done: usize,
    pub(crate) failed: usize,
    pub(crate) cancelled: usize,
    pub(crate) interrupted: usize,
    /// 当前下载子行在整批里的 1-based 位置（没有子行在下载时为 None）
    pub(crate) running_position: Option<usize>,
    pub(crate) running_name: String,
    /// 已完成子行字节 + 当前子行已下载字节（失败 / 取消的残量不计入）
    pub(crate) progress_bytes: u64,
    /// 所有子行总大小都已知时才有值
    pub(crate) total_bytes: Option<u64>,
    known_total_bytes: u64,
    unknown_totals: usize,
    child_indices: BTreeSet<usize>,
    running_children: BTreeMap<usize, String>,
}

impl BatchStats {
    pub(crate) fn finished(&self) -> usize {
        self.done + self.failed + self.cancelled
    }

    fn add_child(&mut self, child: ChildStats) {
        self.total += 1;
        self.child_indices.insert(child.index);
        self.state_count_mut(child.state, 1);
        self.progress_bytes = self
            .progress_bytes
            .saturating_add(progress_contribution(child.state, child.bytes));
        match child.total {
            Some(total) => self.known_total_bytes = self.known_total_bytes.saturating_add(total),
            None => self.unknown_totals += 1,
        }
        if child.state == TransferState::Running {
            self.running_children.insert(child.index, child.name);
        }
        self.refresh_derived();
    }

    fn remove_child(&mut self, child: ChildStats) {
        self.total = self.total.saturating_sub(1);
        self.child_indices.remove(&child.index);
        self.state_count_mut(child.state, -1);
        self.progress_bytes = self
            .progress_bytes
            .saturating_sub(progress_contribution(child.state, child.bytes));
        match child.total {
            Some(total) => {
                self.known_total_bytes = self.known_total_bytes.saturating_sub(total);
            }
            None => self.unknown_totals = self.unknown_totals.saturating_sub(1),
        }
        if child.state == TransferState::Running {
            self.running_children.remove(&child.index);
        }
        self.refresh_derived();
    }

    fn update_child(&mut self, before: ChildStats, after: ChildStats) {
        if before.index != after.index {
            self.remove_child(before);
            self.add_child(after);
            return;
        }
        self.progress_bytes = self
            .progress_bytes
            .saturating_sub(progress_contribution(before.state, before.bytes))
            .saturating_add(progress_contribution(after.state, after.bytes));
        if before.state != after.state {
            self.state_count_mut(before.state, -1);
            self.state_count_mut(after.state, 1);
            if before.state == TransferState::Running {
                self.running_children.remove(&before.index);
            }
            if after.state == TransferState::Running {
                self.running_children
                    .insert(after.index, after.name.clone());
            }
        }
        if before.total != after.total {
            if let Some(total) = before.total {
                self.known_total_bytes = self.known_total_bytes.saturating_sub(total);
            } else {
                self.unknown_totals = self.unknown_totals.saturating_sub(1);
            }
            if let Some(total) = after.total {
                self.known_total_bytes = self.known_total_bytes.saturating_add(total);
            } else {
                self.unknown_totals += 1;
            }
        }
        if after.state == TransferState::Running && before.name != after.name {
            self.running_children
                .insert(after.index, after.name.clone());
        }
        if before.state != after.state || before.total != after.total {
            self.refresh_derived();
        } else if after.state == TransferState::Running && before.name != after.name {
            self.refresh_running();
        }
    }

    fn state_count_mut(&mut self, state: TransferState, delta: i64) {
        let slot = match state {
            TransferState::Queued => &mut self.queued,
            TransferState::Preparing => &mut self.preparing,
            TransferState::Running => &mut self.running,
            TransferState::Pausing => &mut self.pausing,
            TransferState::Paused => &mut self.paused,
            TransferState::Done => &mut self.done,
            TransferState::Failed => &mut self.failed,
            TransferState::Cancelled => &mut self.cancelled,
            TransferState::Interrupted => &mut self.interrupted,
        };
        if delta >= 0 {
            *slot = slot.saturating_add(delta as usize);
        } else {
            *slot = slot.saturating_sub(delta.unsigned_abs() as usize);
        }
    }

    fn refresh_derived(&mut self) {
        self.total_bytes = (self.unknown_totals == 0).then_some(self.known_total_bytes);
        self.refresh_running();
    }

    fn refresh_running(&mut self) {
        let first = self.running_children.iter().next();
        self.running_position =
            first.map(|(index, _)| self.child_indices.range(..*index).count().saturating_add(1));
        self.running_name = first.map(|(_, name)| name.clone()).unwrap_or_default();
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ChildStats {
    pub(crate) index: usize,
    pub(crate) state: TransferState,
    pub(crate) bytes: u64,
    pub(crate) total: Option<u64>,
    pub(crate) name: String,
}

impl ChildStats {
    pub(crate) fn of(record: &Transfer) -> Self {
        Self {
            index: record.index,
            state: record.state,
            bytes: display_bytes(record),
            total: record.total,
            name: record.job.name.clone(),
        }
    }
}

/// 文件夹父行聚合用：上传中的在途字节也要算进去；其它状态仍用可续传字节。
pub(crate) fn display_bytes(record: &Transfer) -> u64 {
    if record.state == TransferState::Running && record.job.is_upload() {
        if let Some(progress) = record.upload_progress {
            if progress.stage == UploadStage::Uploading {
                return progress.bytes;
            }
        }
    }
    record.bytes
}

pub(crate) fn progress_contribution(state: TransferState, bytes: u64) -> u64 {
    match state {
        TransferState::Done
        | TransferState::Running
        | TransferState::Pausing
        | TransferState::Paused => bytes,
        TransferState::Queued
        | TransferState::Preparing
        | TransferState::Failed
        | TransferState::Cancelled
        | TransferState::Interrupted => 0,
    }
}

/// 聚合一个批次的所有子行（按 index 顺序；子行下载顺序固定）。
///
/// 生产路径读取父行缓存的 [`BatchStats`]；这个纯函数只用于初始化、整批
/// 状态迁移和单测，避免在每次完成一个文件时重新扫全批。
pub(crate) fn batch_stats(records: &[Transfer], batch: u64) -> BatchStats {
    let mut children: Vec<&Transfer> = records
        .iter()
        .filter(|record| record.batch == batch && record.child)
        .collect();
    children.sort_by_key(|record| record.index);

    let mut stats = BatchStats::default();
    for child in children {
        stats.add_child(ChildStats::of(child));
    }
    stats
}

/// 整批状态迁移（暂停 / 取消 / 中断等）后只重算这一次；文件完成仍走
/// [`update_child_stats`] 的增量路径。
pub(crate) fn rebuild_batch_stats(records: &mut [Transfer], batch: u64) {
    let stats = batch_stats(records, batch);
    if let Some(parent) = records
        .iter_mut()
        .find(|record| record.batch == batch && is_folder_ledger(record))
    {
        parent.batch_stats = stats;
    }
}

pub(crate) fn rebuild_all_batch_stats(records: &mut [Transfer]) {
    let mut stats: HashMap<u64, BatchStats> = HashMap::new();
    for child in records.iter().filter(|record| record.child) {
        stats
            .entry(child.batch)
            .or_default()
            .add_child(ChildStats::of(child));
    }
    for record in records.iter_mut().filter(|record| is_folder_ledger(record)) {
        record.batch_stats = stats.remove(&record.batch).unwrap_or_default();
    }
}

pub(crate) fn update_child_stats(
    records: &mut [Transfer],
    batch: u64,
    before: ChildStats,
    after: ChildStats,
) {
    if let Some(parent) = records
        .iter_mut()
        .find(|record| record.batch == batch && is_folder_ledger(record))
    {
        parent.batch_stats.update_child(before, after);
    }
}

/// 文件夹批次的总账行状态由子行聚合而来。
///
/// 取消 / 已中断后不再被后续完成事件改回（两者的优先级最高）；没有子行时不动父行。
pub(crate) fn refresh_parent_state(records: &mut [Transfer], batch: u64) {
    let Some(parent) = records
        .iter()
        .position(|record| record.batch == batch && is_folder_ledger(record))
    else {
        return;
    };
    let parent_state = records[parent].state;
    if matches!(
        parent_state,
        TransferState::Cancelled | TransferState::Interrupted
    ) {
        return;
    }
    let failed_dirs = records[parent].upload_failed_dirs.len();
    let (children, active, running, pausing, paused, failed, cancelled, interrupted, all_done) = {
        let stats = &records[parent].batch_stats;
        (
            stats.total,
            stats.queued + stats.preparing + stats.running > 0,
            stats.running > 0,
            stats.pausing > 0,
            stats.paused > 0,
            stats.failed > 0,
            stats.cancelled > 0,
            stats.interrupted > 0,
            stats.total > 0 && stats.done == stats.total,
        )
    };
    if children == 0 {
        if failed_dirs > 0 {
            records[parent].state = TransferState::Failed;
        }
        return;
    }

    records[parent].state = if pausing {
        TransferState::Pausing
    } else if active {
        // 读目录刚结束、子行还没开始时保持「准备中」
        if running || parent_state != TransferState::Preparing {
            TransferState::Running
        } else {
            TransferState::Preparing
        }
    } else if paused {
        TransferState::Paused
    } else if interrupted {
        TransferState::Interrupted
    } else if failed || failed_dirs > 0 {
        TransferState::Failed
    } else if all_done {
        TransferState::Done
    } else if cancelled {
        TransferState::Cancelled
    } else {
        TransferState::Done
    };
}
