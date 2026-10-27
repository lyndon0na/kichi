//! 传输记录的 QML 视图映射：一条记录 → 一行 `QVariantMap`，以及排序 / 快照。
//!
//! 从 `transfers.rs` 拆出来守行数上限：那边管状态机与记录表，这里只管
//! 「字段叫什么、什么时候显示什么、按什么顺序交给 QML」。角色名不能叫 `state`
//! （`Item` 自带同名属性，见 `docs/pitfalls.md` 第 12 条），QML 侧统一映射成 `phase`。
//!
//! 文件夹批次在主列表只有一条总账行：进度 / 大小 / 文件数 / 失败数都由
//! 子行聚合而来；子行本身只进文件夹详情页的快照。

use cxx_qt_lib::{QMap, QMapPair_QString_QVariant, QString, QVariant};

use super::actions::format_size;
use super::transfers::{
    folder_download_path, is_folder_ledger, record_key, Transfer, TransferState,
};
use super::QVariantList;

/// 一次最多交给 QML 的行数（大文件夹下载时避免整列表反复重建拖慢界面）。
pub(crate) const MAX_ROWS: usize = 500;

pub(crate) type QVariantMap = QMap<QMapPair_QString_QVariant>;

pub(crate) fn percent_of(record: &Transfer) -> Option<i32> {
    match record.total {
        Some(total) if total > 0 => {
            Some(((record.bytes as f64 / total as f64) * 100.0).clamp(0.0, 100.0) as i32)
        }
        _ => None,
    }
}

fn text(value: &str) -> QVariant {
    QVariant::from(&QString::from(value))
}

fn insert(map: &mut QVariantMap, key: &str, value: QVariant) {
    map.insert_clone(&QString::from(key), &value);
}

/// 一个文件夹批次的子行聚合。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct BatchStats {
    pub(crate) total: usize,
    pub(crate) done: usize,
    pub(crate) failed: usize,
    pub(crate) cancelled: usize,
    /// 当前下载子行在整批里的 1-based 位置（没有子行在下载时为 None）
    pub(crate) running_position: Option<usize>,
    pub(crate) running_name: String,
    /// 已完成子行字节 + 当前子行已下载字节（失败 / 取消的残量不计入）
    pub(crate) progress_bytes: u64,
    /// 所有子行总大小都已知时才有值
    pub(crate) total_bytes: Option<u64>,
}

impl BatchStats {
    pub(crate) fn finished(&self) -> usize {
        self.done + self.failed + self.cancelled
    }
}

/// 聚合一个批次的所有子行（按 index 顺序；子行下载顺序固定）。
pub(crate) fn batch_stats(records: &[Transfer], batch: u64) -> BatchStats {
    let mut children: Vec<&Transfer> = records
        .iter()
        .filter(|record| record.batch == batch && record.child)
        .collect();
    children.sort_by_key(|record| record.index);

    let mut stats = BatchStats::default();
    let mut total_bytes = 0u64;
    let mut sizes_known = true;
    for child in children {
        stats.total += 1;
        match child.state {
            TransferState::Done => {
                stats.done += 1;
                stats.progress_bytes += child.bytes;
            }
            TransferState::Running => {
                stats.progress_bytes += child.bytes;
                if stats.running_position.is_none() {
                    stats.running_position = Some(stats.total);
                    stats.running_name = child.job.name.clone();
                }
            }
            TransferState::Failed => stats.failed += 1,
            TransferState::Cancelled => stats.cancelled += 1,
            TransferState::Queued | TransferState::Preparing => {}
        }
        match child.total {
            // 0 字节子行按 0 处理，不参与「总量未知」判断
            Some(size) => total_bytes += size,
            None => sizes_known = false,
        }
    }
    stats.total_bytes = sizes_known.then_some(total_bytes);
    stats
}

fn percent_bytes(stats: &BatchStats) -> Option<i32> {
    let total = stats.total_bytes?;
    if total == 0 {
        return Some(0);
    }
    Some(((stats.progress_bytes as f64 / total as f64) * 100.0).clamp(0.0, 100.0) as i32)
}

/// 文件夹总账行的进度条 / 大小 / 百分比文案（主列表与详情页共用）。
fn folder_progress_fields(stats: &BatchStats, state: TransferState) -> (i32, String, String) {
    let percent = percent_bytes(stats);
    let progress = match state {
        TransferState::Done => 100,
        TransferState::Running => percent.unwrap_or(-1),
        _ => percent.unwrap_or(0),
    };
    let position = stats
        .running_position
        .unwrap_or_else(|| (stats.finished() + 1).min(stats.total));
    let progress_text = if state == TransferState::Running && stats.total > 0 {
        match percent {
            Some(value) => format!("{value}% · {position}/{}", stats.total),
            None => format!("{position}/{}", stats.total),
        }
    } else {
        String::new()
    };
    let size_text = match state {
        TransferState::Running => match stats.total_bytes {
            Some(total) => format!(
                "{} / {}",
                format_size(stats.progress_bytes),
                format_size(total)
            ),
            None => format_size(stats.progress_bytes),
        },
        TransferState::Done => format_size(stats.total_bytes.unwrap_or(stats.progress_bytes)),
        TransferState::Preparing => stats
            .total_bytes
            .map(format_size)
            .unwrap_or_else(|| "—".to_owned()),
        _ => stats
            .total_bytes
            .map(format_size)
            .or_else(|| (stats.progress_bytes > 0).then(|| format_size(stats.progress_bytes)))
            .unwrap_or_else(|| "—".to_owned()),
    };
    (progress, progress_text, size_text)
}

/// 文件夹总账行的第二行说明（不含「点开可重试」，那由 QML 追加）。
fn folder_detail(record: &Transfer, stats: &BatchStats, has_children: bool) -> String {
    if !has_children {
        return match record.state {
            TransferState::Preparing => "正在读取目录结构…".to_owned(),
            _ => record.detail.clone(),
        };
    }
    match record.state {
        TransferState::Queued => String::new(),
        TransferState::Preparing => format!("准备下载 {} 个文件…", stats.total),
        TransferState::Running => {
            let position = stats
                .running_position
                .unwrap_or_else(|| (stats.finished() + 1).min(stats.total));
            format!("第 {position}/{} 个", stats.total)
        }
        TransferState::Done => match folder_download_path(record) {
            Some(path) => format!("{} 个文件 · 保存到 {}", stats.total, path.display()),
            None => format!("{} 个文件", stats.total),
        },
        TransferState::Failed => {
            let mut text = format!("成功 {} 个 · 失败 {} 个", stats.done, stats.failed);
            if stats.cancelled > 0 {
                text.push_str(&format!(" · 取消 {} 个", stats.cancelled));
            }
            text
        }
        TransferState::Cancelled => {
            let mut text = format!("已完成 {} 个 · 已取消", stats.done);
            if stats.failed > 0 {
                text.push_str(&format!(" · 失败 {} 个", stats.failed));
            }
            text
        }
    }
}

fn file_record_map(record: &Transfer, name: String) -> QVariantMap {
    let percent = percent_of(record);
    let running = record.state == TransferState::Running;
    let progress = match record.state {
        TransferState::Done => 100,
        TransferState::Running => percent.unwrap_or(-1),
        TransferState::Failed | TransferState::Cancelled => percent.unwrap_or(0),
        TransferState::Queued | TransferState::Preparing => 0,
    };
    let progress_text = if running {
        percent
            .map(|value| format!("{value}%"))
            .unwrap_or_else(|| format_size(record.bytes))
    } else {
        String::new()
    };
    let size_text = match record.state {
        TransferState::Running => match record.total {
            Some(total) => format!("{} / {}", format_size(record.bytes), format_size(total)),
            None => format_size(record.bytes),
        },
        TransferState::Done => format_size(record.total.unwrap_or(record.bytes).max(record.bytes)),
        // 准备中还没读目录，整批多大都不知道
        TransferState::Preparing => "—".to_owned(),
        _ => record
            .total
            .map(format_size)
            .or_else(|| (record.bytes > 0).then(|| format_size(record.bytes)))
            .unwrap_or_else(|| "—".to_owned()),
    };
    // 完成行的说明由落盘路径派生；准备中的说明行直接由状态决定
    let detail = match record.state {
        TransferState::Preparing => "正在读取目录结构…".to_owned(),
        TransferState::Done => record
            .path
            .as_ref()
            .map(|path| format!("保存到 {}", path.display()))
            .unwrap_or_default(),
        _ => record.detail.clone(),
    };
    let path_text = record
        .path
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_default();

    let mut map = QVariantMap::default();
    insert(&mut map, "key", text(&record_key(record)));
    insert(&mut map, "name", text(&name));
    insert(&mut map, "sourceId", text(&record.source_id));
    insert(&mut map, "state", text(record.state.name()));
    insert(&mut map, "stateText", text(record.state.text()));
    insert(&mut map, "running", QVariant::from(&running));
    insert(&mut map, "progress", QVariant::from(&progress));
    insert(&mut map, "progressText", text(&progress_text));
    insert(&mut map, "sizeText", text(&size_text));
    insert(&mut map, "detail", text(&detail));
    insert(&mut map, "path", text(&path_text));
    insert(&mut map, "folder", QVariant::from(&false));
    insert(&mut map, "childCount", QVariant::from(&0i32));
    insert(&mut map, "failedCount", QVariant::from(&0i32));
    map
}

fn folder_record_map(record: &Transfer, records: &[Transfer]) -> QVariantMap {
    let stats = batch_stats(records, record.batch);
    let has_children = stats.total > 0;
    let partial = record.state == TransferState::Failed && has_children && stats.done > 0;
    let state_text = if partial {
        "部分失败".to_owned()
    } else {
        record.state.text().to_owned()
    };
    let running = record.state == TransferState::Running;
    let (progress, progress_text, size_text) = if has_children {
        folder_progress_fields(&stats, record.state)
    } else {
        let progress = if record.state == TransferState::Done {
            100
        } else {
            0
        };
        (progress, String::new(), "—".to_owned())
    };
    let detail = folder_detail(record, &stats, has_children);
    let path_text = if has_children && stats.done > 0 {
        folder_download_path(record)
            .map(|path| path.display().to_string())
            .unwrap_or_default()
    } else {
        String::new()
    };

    let mut map = QVariantMap::default();
    insert(&mut map, "key", text(&record_key(record)));
    insert(&mut map, "name", text(&record.job.name));
    insert(&mut map, "sourceId", text(&record.source_id));
    insert(&mut map, "state", text(record.state.name()));
    insert(&mut map, "stateText", text(&state_text));
    insert(&mut map, "running", QVariant::from(&running));
    insert(&mut map, "progress", QVariant::from(&progress));
    insert(&mut map, "progressText", text(&progress_text));
    insert(&mut map, "sizeText", text(&size_text));
    insert(&mut map, "detail", text(&detail));
    insert(&mut map, "path", text(&path_text));
    insert(&mut map, "folder", QVariant::from(&true));
    insert(
        &mut map,
        "childCount",
        QVariant::from(&i32::try_from(stats.total).unwrap_or(i32::MAX)),
    );
    insert(
        &mut map,
        "failedCount",
        QVariant::from(&i32::try_from(stats.failed).unwrap_or(i32::MAX)),
    );
    map
}

/// 一条记录 → 主列表的一行（文件夹总账行会聚合子行）。
pub(crate) fn record_map(record: &Transfer, records: &[Transfer]) -> QVariantMap {
    if is_folder_ledger(record) {
        folder_record_map(record, records)
    } else {
        file_record_map(record, record.display_name())
    }
}

/// 一条子行 → 详情页的一行：名字用相对文件夹的路径（去掉第一段文件夹名）。
fn detail_record_map(record: &Transfer) -> QVariantMap {
    let mut parts = record.job.sub_dir.iter();
    parts.next();
    let rest: Vec<&str> = parts.map(String::as_str).collect();
    let name = if rest.is_empty() {
        record.job.name.clone()
    } else {
        format!("{}/{}", rest.join("/"), record.job.name)
    };
    file_record_map(record, name)
}

/// 一条记录 → 进度行的 map（只带下载中要刷新的字段）。
pub(crate) fn live_map(record: &Transfer) -> QVariantMap {
    let percent = percent_of(record);
    let progress_text = percent
        .map(|value| format!("{value}%"))
        .unwrap_or_else(|| format_size(record.bytes));
    let size_text = match record.total {
        Some(total) => format!("{} / {}", format_size(record.bytes), format_size(total)),
        None => format_size(record.bytes),
    };

    let mut map = QVariantMap::default();
    insert(&mut map, "key", text(&record_key(record)));
    insert(&mut map, "progress", QVariant::from(&percent.unwrap_or(-1)));
    insert(&mut map, "progressText", text(&progress_text));
    insert(&mut map, "sizeText", text(&size_text));
    map
}

/// 文件夹父行的实时聚合（字节数取当前最新值，只带刷新用到的字段）。
pub(crate) fn parent_live_map(record: &Transfer, records: &[Transfer]) -> QVariantMap {
    let stats = batch_stats(records, record.batch);
    let (progress, progress_text, size_text) =
        folder_progress_fields(&stats, TransferState::Running);

    let mut map = QVariantMap::default();
    insert(&mut map, "key", text(&record_key(record)));
    insert(&mut map, "progress", QVariant::from(&progress));
    insert(&mut map, "progressText", text(&progress_text));
    insert(&mut map, "sizeText", text(&size_text));
    map
}

fn notice(key: &str, message: &str) -> QVariantMap {
    let mut map = QVariantMap::default();
    insert(&mut map, "key", text(key));
    insert(&mut map, "name", text(message));
    insert(&mut map, "sourceId", text(""));
    insert(&mut map, "state", text("notice"));
    insert(&mut map, "stateText", text(""));
    insert(&mut map, "running", QVariant::from(&false));
    insert(&mut map, "progress", QVariant::from(&0));
    insert(&mut map, "progressText", text(""));
    insert(&mut map, "sizeText", text(""));
    insert(&mut map, "detail", text(""));
    insert(&mut map, "path", text(""));
    insert(&mut map, "folder", QVariant::from(&false));
    insert(&mut map, "childCount", QVariant::from(&0i32));
    insert(&mut map, "failedCount", QVariant::from(&0i32));
    map
}

/// 超过显示上限时追加的提示行（ListView 里最后一行）。
///
/// **必须给 `key`**：QML 的差异算法靠它识别行；缺了会让 ListModel 报
/// `<Unknown File>: key is undefined`（2026-10-27 实测），提示行也会在每次
/// 刷新时被当成新行重复插入。
pub(crate) fn notice_map(hidden: usize) -> QVariantMap {
    notice(
        "notice",
        &format!("还有 {hidden} 条记录未显示（页面只显示前 500 条）"),
    )
}

fn detail_notice_map(hidden: usize) -> QVariantMap {
    notice(
        "notice",
        &format!("还有 {hidden} 个文件未显示（明细只显示前 500 个）"),
    )
}

/// 排序：进行中在前（批次内按顺序），已结束在后（新 → 旧）。只取顶层记录。
pub(crate) fn ordered(records: &[Transfer]) -> Vec<&Transfer> {
    let mut active: Vec<&Transfer> = records
        .iter()
        .filter(|record| !record.child && record.state.is_active())
        .collect();
    let mut finished: Vec<&Transfer> = records
        .iter()
        .filter(|record| !record.child && record.state.is_finished())
        .collect();
    active.sort_by_key(|record| (record.batch, record.index));
    finished.sort_by(|a, b| (b.batch, b.index).cmp(&(a.batch, a.index)));
    active.extend(finished);
    active
}

/// 记录表 → 主列表模型 + 进行中条数（按批次算，一个文件夹算 1）。
pub(crate) fn build_snapshot(records: &[Transfer]) -> (QVariantList, i32) {
    let top_level: Vec<&Transfer> = records.iter().filter(|record| !record.child).collect();
    let total = top_level.len();
    let active = top_level
        .iter()
        .filter(|record| record.state.is_active())
        .count();
    let mut list = QVariantList::default();
    for record in ordered(records).into_iter().take(MAX_ROWS) {
        list.append(QVariant::from(&record_map(record, records)));
    }
    if total > MAX_ROWS {
        list.append(QVariant::from(&notice_map(total - MAX_ROWS)));
    }
    let active = i32::try_from(active).unwrap_or(i32::MAX);
    (list, active)
}

/// 记录表 → 某个文件夹批次的子文件快照（固定 index 顺序，不按状态重排）。
pub(crate) fn build_detail_snapshot(records: &[Transfer], batch: u64) -> QVariantList {
    let mut children: Vec<&Transfer> = records
        .iter()
        .filter(|record| record.batch == batch && record.child)
        .collect();
    children.sort_by_key(|record| record.index);
    let total = children.len();
    let mut list = QVariantList::default();
    for record in children.into_iter().take(MAX_ROWS) {
        list.append(QVariant::from(&detail_record_map(record)));
    }
    if total > MAX_ROWS {
        list.append(QVariant::from(&detail_notice_map(total - MAX_ROWS)));
    }
    list
}
