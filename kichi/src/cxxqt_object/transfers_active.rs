//! 传输页顶部「进行中」摘要：把正在跑的单文件或文件夹批次映射成一张卡片。
//!
//! 这里只读记录表，不改变调度或记录状态。独立文件按「不同来源」聚合；文件夹
//! 批次用文件夹名作标题，卡片里最多列 3 条当前正在下载的子文件。分片属于同一
//! 文件内部连接，不会在摘要里拆成多行。

use std::time::Instant;

use cxx_qt_lib::{QList, QVariant};
use kichi_core::EntryKind;

use super::actions::format_size;
use super::transfers::{is_folder_ledger, record_key, with_records, Transfer, TransferState};
use super::transfers_rate::file_rate_info;
use super::transfers_view::{batch_stats, insert, percent_of, text, QVariantMap};
use super::QVariantList;

const MAX_VISIBLE_LANES: usize = 3;

pub(crate) fn snapshot() -> QVariantList {
    with_records(|records| snapshot_at(records, Instant::now()))
}

pub(crate) fn snapshot_at(records: &[Transfer], now: Instant) -> QVariantList {
    let folder = records.iter().find(|record| {
        is_folder_ledger(record)
            && matches!(
                record.state,
                TransferState::Preparing | TransferState::Running
            )
    });
    let group = match folder {
        Some(record) => folder_group(records, record, now),
        None => independent_group(records, now),
    };
    let mut list = QVariantList::default();
    if let Some(group) = group {
        list.append(QVariant::from(&group));
    }
    list
}

fn lane_name(record: &Transfer, strip_folder_root: bool) -> String {
    if !strip_folder_root {
        return record.display_name();
    }
    let mut parts = record.job.sub_dir.iter();
    parts.next();
    let rest: Vec<&str> = parts.map(String::as_str).collect();
    if rest.is_empty() {
        record.job.name.clone()
    } else {
        format!("{}/{}", rest.join("/"), record.job.name)
    }
}

fn lane_map(record: &Transfer, now: Instant, strip_folder_root: bool) -> QVariantMap {
    let percent = percent_of(record);
    let rate = file_rate_info(record, now);
    let size_text = match record.total {
        Some(total) => format!("{} / {}", format_size(record.bytes), format_size(total)),
        None => format_size(record.bytes),
    };

    let mut map = QVariantMap::default();
    insert(&mut map, "key", text(&record_key(record)));
    insert(&mut map, "sourceId", text(&record.source_id));
    insert(
        &mut map,
        "name",
        text(&lane_name(record, strip_folder_root)),
    );
    insert(&mut map, "progress", QVariant::from(&percent.unwrap_or(-1)));
    insert(
        &mut map,
        "progressText",
        text(
            &percent
                .map(|value| format!("{value}%"))
                .unwrap_or_else(|| format_size(record.bytes)),
        ),
    );
    insert(&mut map, "sizeText", text(&size_text));
    insert(&mut map, "rateText", text(&rate.rate));
    insert(&mut map, "noteText", text(&rate.note));
    // 记录表目前不保存实际分片计划，先留稳定空字段；将来拿到真实元数据再填。
    insert(&mut map, "shardText", text(""));
    map
}

fn folder_group(records: &[Transfer], folder: &Transfer, now: Instant) -> Option<QVariantMap> {
    let stats = batch_stats(records, folder.batch);
    let running: Vec<&Transfer> = records
        .iter()
        .filter(|record| {
            record.batch == folder.batch && record.child && record.state == TransferState::Running
        })
        .collect();
    let mut lanes = QList::<QVariant>::default();
    for record in running.iter().take(MAX_VISIBLE_LANES) {
        lanes.append(QVariant::from(&lane_map(record, now, true)));
    }
    let progress = match folder.state {
        TransferState::Preparing => -1,
        _ => match stats.total_bytes {
            Some(0) => 0,
            Some(total) => {
                ((stats.progress_bytes as f64 / total as f64) * 100.0).clamp(0.0, 100.0) as i32
            }
            None => -1,
        },
    };
    let size_text = match stats.total_bytes {
        Some(total) => format!(
            "{} / {}",
            format_size(stats.progress_bytes),
            format_size(total)
        ),
        None => format_size(stats.progress_bytes),
    };

    let mut group = QVariantMap::default();
    insert(&mut group, "kind", text("folder"));
    insert(&mut group, "groupKey", text(&record_key(folder)));
    insert(&mut group, "sourceId", text(&folder.source_id));
    insert(&mut group, "name", text(&folder.job.name));
    insert(&mut group, "state", text(folder.state.name()));
    insert(&mut group, "stateText", text(folder.state.text()));
    insert(
        &mut group,
        "runningCount",
        QVariant::from(&i32::try_from(running.len()).unwrap_or(i32::MAX)),
    );
    insert(
        &mut group,
        "totalCount",
        QVariant::from(&i32::try_from(stats.total).unwrap_or(i32::MAX)),
    );
    insert(
        &mut group,
        "finishedCount",
        QVariant::from(&i32::try_from(stats.finished()).unwrap_or(i32::MAX)),
    );
    insert(
        &mut group,
        "hiddenCount",
        QVariant::from(
            &i32::try_from(running.len().saturating_sub(MAX_VISIBLE_LANES)).unwrap_or(i32::MAX),
        ),
    );
    insert(&mut group, "progress", QVariant::from(&progress));
    insert(&mut group, "sizeText", text(&size_text));
    insert(
        &mut group,
        "progressText",
        text(&if stats.total > 0 {
            format!("{}/{} 个文件", stats.finished(), stats.total)
        } else {
            String::new()
        }),
    );
    insert(&mut group, "lanes", QVariant::from(&lanes));
    Some(group)
}

fn independent_group(records: &[Transfer], now: Instant) -> Option<QVariantMap> {
    let running: Vec<&Transfer> = records
        .iter()
        .filter(|record| {
            !record.child
                && record.job.kind == EntryKind::File
                && record.state == TransferState::Running
        })
        .collect();
    if running.is_empty() {
        return None;
    }

    let mut lanes = QList::<QVariant>::default();
    for record in running.iter().take(MAX_VISIBLE_LANES) {
        lanes.append(QVariant::from(&lane_map(record, now, false)));
    }
    let progress_bytes: u64 = running.iter().map(|record| record.bytes).sum();
    let total_bytes = running
        .iter()
        .try_fold(0u64, |sum, record| Some(sum.saturating_add(record.total?)));
    let progress = total_bytes
        .filter(|total| *total > 0)
        .map(|total| ((progress_bytes as f64 / total as f64) * 100.0).clamp(0.0, 100.0) as i32)
        .unwrap_or(-1);
    let size_text = match total_bytes {
        Some(total) => format!("{} / {}", format_size(progress_bytes), format_size(total)),
        None => format_size(progress_bytes),
    };

    let mut group = QVariantMap::default();
    insert(&mut group, "kind", text("files"));
    insert(&mut group, "groupKey", text("files"));
    insert(&mut group, "sourceId", text(""));
    insert(&mut group, "name", text(""));
    insert(&mut group, "state", text("running"));
    insert(&mut group, "stateText", text("下载中"));
    insert(
        &mut group,
        "runningCount",
        QVariant::from(&i32::try_from(running.len()).unwrap_or(i32::MAX)),
    );
    insert(
        &mut group,
        "totalCount",
        QVariant::from(&i32::try_from(running.len()).unwrap_or(i32::MAX)),
    );
    insert(&mut group, "finishedCount", QVariant::from(&0i32));
    insert(
        &mut group,
        "hiddenCount",
        QVariant::from(
            &i32::try_from(running.len().saturating_sub(MAX_VISIBLE_LANES)).unwrap_or(i32::MAX),
        ),
    );
    insert(&mut group, "progress", QVariant::from(&progress));
    insert(&mut group, "sizeText", text(&size_text));
    insert(&mut group, "progressText", text(""));
    insert(&mut group, "lanes", QVariant::from(&lanes));
    Some(group)
}

#[cfg(test)]
#[path = "transfers_active_tests.rs"]
mod tests;
