//! 传输记录的 QML 视图映射：一条记录 → 一行 `QVariantMap`，以及排序 / 快照。
//!
//! 从 `transfers.rs` 拆出来守行数上限：那边管状态机与记录表，这里只管
//! 「字段叫什么、什么时候显示什么、按什么顺序交给 QML」。角色名不能叫 `state`
//! （`Item` 自带同名属性，见 `docs/pitfalls.md` 第 12 条），QML 侧统一映射成 `phase`。
//!
//! 文件夹批次在主列表只有一条总账行：进度 / 大小 / 文件数 / 失败数都由
//! 子行聚合而来；子行本身只进文件夹详情页的快照。

use std::time::Instant;

use chrono::TimeZone;
use cxx_qt_lib::{QMap, QMapPair_QString_QVariant, QString, QVariant};

use super::actions::format_size;
use super::transfers::{
    folder_download_path, is_folder_ledger, record_key, Transfer, TransferState,
};
use super::transfers_rate::{batch_rate_info, file_rate_info};
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
            // 上次没下完：既不算成功也不算失败，父行保持「已中断」
            TransferState::Interrupted => {}
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
///
/// 已中断的父行单独处理：它的说明是「上次退出时未完成 · 完成 x 个 · 失败 y 个」，
/// 不参与下面按状态拼的文案（那些都是当前这一轮才有的结果）。
fn folder_detail(record: &Transfer, stats: &BatchStats, has_children: bool) -> String {
    let skipped = skip_suffix(record.skipped);
    if record.state == TransferState::Interrupted && !record.detail.is_empty() {
        // 读目录前就中断：原因由历史文件带来
        return record.detail.clone();
    }
    if record.state == TransferState::Interrupted {
        let mut text = if has_children {
            format!(
                "上次退出时未完成 · 完成 {} 个 · 失败 {} 个",
                stats.done, stats.failed
            )
        } else {
            "上次退出时未完成".to_owned()
        };
        text.push_str(&skipped);
        return text;
    }
    if !has_children {
        // 空文件夹没有子行：完成说明直接由落盘路径派生
        return match record.state {
            TransferState::Preparing => "正在读取目录结构…".to_owned(),
            TransferState::Done if record.empty_folder => {
                let mut text = match folder_download_path(record) {
                    Some(path) => format!("空文件夹 · 保存到 {}", path.display()),
                    None => "空文件夹".to_owned(),
                };
                text.push_str(&skipped);
                text
            }
            _ => record.detail.clone(),
        };
    }
    let mut text = match record.state {
        TransferState::Queued => return String::new(),
        // 准备中还没开始下载，跳过数等进下载循环再显示
        TransferState::Preparing => return format!("准备下载 {} 个文件…", stats.total),
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
        // 上面已经单独返回
        TransferState::Interrupted => "上次退出时未完成".to_owned(),
        TransferState::Failed => {
            let mut detail = format!("成功 {} 个 · 失败 {} 个", stats.done, stats.failed);
            if stats.cancelled > 0 {
                detail.push_str(&format!(" · 取消 {} 个", stats.cancelled));
            }
            detail
        }
        TransferState::Cancelled => {
            let mut detail = format!("已完成 {} 个 · 已取消", stats.done);
            if stats.failed > 0 {
                detail.push_str(&format!(" · 失败 {} 个", stats.failed));
            }
            detail
        }
    };
    text.push_str(&skipped);
    text
}

/// 说明行末尾的跳过后缀（没有跳过时为空串）。
fn skip_suffix(skipped: usize) -> String {
    if skipped == 0 {
        String::new()
    } else {
        format!(" · 跳过 {skipped} 个目录")
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
        // 上次下到一半：进度条停在历史位置，不假装 0
        TransferState::Interrupted => percent.unwrap_or(0),
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
    // 完成行的说明由落盘路径派生；准备中 / 已中断的说明行直接由状态决定
    let detail = match record.state {
        TransferState::Preparing => "正在读取目录结构…".to_owned(),
        TransferState::Done => record
            .path
            .as_ref()
            .map(|path| format!("保存到 {}", path.display()))
            .unwrap_or_default(),
        // 已中断：说明由 QML 的「重试」行补断点文案，这里给状态说明
        TransferState::Interrupted if record.detail.is_empty() => "上次退出时未完成".to_owned(),
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
    insert(
        &mut map,
        "createdUnix",
        QVariant::from(&record.created_unix),
    );
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
    // 至少一个子文件完成，或者空文件夹已完成（目录已建出来）：才有落盘路径。
    // 已中断的文件夹若在中断前有子文件落下，同样给路径——那些文件确实在磁盘上。
    let empty_done = record.empty_folder && record.state == TransferState::Done;
    let path_text = if (has_children && stats.done > 0) || empty_done {
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
    insert(
        &mut map,
        "createdUnix",
        QVariant::from(&record.created_unix),
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
///
/// `rateText` / `noteText` 只在这一份里出现：它们每秒都会变，放进
/// `transfers` / `transferDetail` 快照会让 QML 每秒 `setProperty` 整表。
///
/// 生产路径统一走 [`live_map_at`]（一次快照只取一次时钟）；这里转发给单测。
#[cfg(test)]
pub(crate) fn live_map(record: &Transfer) -> QVariantMap {
    live_map_at(record, Instant::now())
}

/// [`live_map`] 的注入时间版本（测试用）。
pub(crate) fn live_map_at(record: &Transfer, now: Instant) -> QVariantMap {
    let percent = percent_of(record);
    let progress_text = percent
        .map(|value| format!("{value}%"))
        .unwrap_or_else(|| format_size(record.bytes));
    let size_text = match record.total {
        Some(total) => format!("{} / {}", format_size(record.bytes), format_size(total)),
        None => format_size(record.bytes),
    };
    let rate = file_rate_info(record, now);

    let mut map = QVariantMap::default();
    insert(&mut map, "key", text(&record_key(record)));
    insert(&mut map, "progress", QVariant::from(&percent.unwrap_or(-1)));
    insert(&mut map, "progressText", text(&progress_text));
    insert(&mut map, "sizeText", text(&size_text));
    insert(&mut map, "rateText", text(&rate.rate));
    insert(&mut map, "noteText", text(&rate.note));
    map
}

/// 文件夹父行的实时聚合（字节数取当前最新值，只带刷新用到的字段）。
///
/// 生产路径统一走 [`parent_live_map_at`]；这里转发给单测。
#[cfg(test)]
pub(crate) fn parent_live_map(record: &Transfer, records: &[Transfer]) -> QVariantMap {
    parent_live_map_at(record, records, Instant::now())
}

/// [`parent_live_map`] 的注入时间版本（测试用）。
pub(crate) fn parent_live_map_at(
    record: &Transfer,
    records: &[Transfer],
    now: Instant,
) -> QVariantMap {
    let stats = batch_stats(records, record.batch);
    let (progress, progress_text, size_text) =
        folder_progress_fields(&stats, TransferState::Running);
    let rate = batch_rate_info(records, record.batch, now);

    let mut map = QVariantMap::default();
    insert(&mut map, "key", text(&record_key(record)));
    insert(&mut map, "progress", QVariant::from(&progress));
    insert(&mut map, "progressText", text(&progress_text));
    insert(&mut map, "sizeText", text(&size_text));
    insert(&mut map, "rateText", text(&rate.rate));
    insert(&mut map, "noteText", text(&rate.note));
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
    insert(&mut map, "createdUnix", QVariant::from(&0i64));
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

/// 排序：进行中 / 已中断在前（批次内按顺序），已结束在后（新 → 旧）。只取顶层记录。
///
/// 已中断的行排最上面：它们不占调度槽，但是用户要先看到、要重试的那几条
/// （ADR 0014：「进行中 / 排队 / 已中断的行不受日期过滤，永远排最上面」）。
pub(crate) fn ordered(records: &[Transfer]) -> Vec<&Transfer> {
    let mut open: Vec<&Transfer> = records
        .iter()
        .filter(|record| {
            !record.child
                && (record.state.is_active() || record.state == TransferState::Interrupted)
        })
        .collect();
    let mut finished: Vec<&Transfer> = records
        .iter()
        .filter(|record| !record.child && record.state.is_finished())
        .collect();
    // 打开着的批次按批号（旧 → 新）；已中断的批次比进行中的还靠前，先让用户看见
    open.sort_by_key(|record| {
        (
            record.state != TransferState::Interrupted,
            record.batch,
            record.index,
        )
    });
    finished.sort_by(|a, b| (b.batch, b.index).cmp(&(a.batch, a.index)));
    open.extend(finished);
    open
}

/// 记录表 → 主列表模型 + 进行中条数（按批次算，一个文件夹算 1），不分日期。
///
/// 生产路径走 [`build_history_snapshot`]（带日期分组）；这个不分组的版本留给单测。
#[cfg(test)]
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

// —— 日期分组（`docs/adr/0014-download-history-on-disk.md`）——

/// 分组查询的「不限日期」。
pub(crate) const HISTORY_DAY_ALL: &str = "all";
/// 分组查询的「今天」（默认；刷新时换算成此刻的日期）。
pub(crate) const HISTORY_DAY_TODAY: &str = "today";

/// 一条记录该归到哪一天（本地时区），`YYYY-MM-DD`。
pub(crate) fn day_key(created_unix: i64) -> String {
    match chrono::Local.timestamp_opt(created_unix, 0).single() {
        Some(time) => time.format("%Y-%m-%d").to_string(),
        None => String::new(),
    }
}

fn day_label(key: &str, today: &str, yesterday: &str) -> String {
    if key == today {
        "今天".to_owned()
    } else if key == yesterday {
        "昨天".to_owned()
    } else {
        // 同年只显示「月-日」；跨年补两位年份，免得「01-02」看不出是哪一年
        let short = key.get(5..).unwrap_or(key);
        match (key.get(..4), today.get(..4)) {
            (Some(year), Some(current)) if year != current => {
                format!("{}-{short}", &year[2..])
            }
            _ => short.to_owned(),
        }
    }
}

/// 历史上出现过的日子（新 → 旧），带显示文案。
///
/// 只统计顶层行（一个批次一行）。`created_unix == 0` 的旧记录不参与分组：
/// 它们进不了任何一天，只能靠「全部」看。
pub(crate) fn history_days(records: &[Transfer], now: i64) -> Vec<(String, String)> {
    let mut keys: Vec<String> = records
        .iter()
        .filter(|record| !record.child && record.created_unix > 0)
        .map(|record| day_key(record.created_unix))
        .filter(|key| !key.is_empty())
        .collect();
    keys.sort();
    keys.dedup();
    keys.reverse();

    let today = day_key(now);
    let yesterday = day_key(now.saturating_sub(24 * 3600));
    keys.into_iter()
        .map(|key| {
            let label = day_label(&key, &today, &yesterday);
            (key, label)
        })
        .collect()
}

/// 这个批次属于哪一天（顶层行的入队时刻）。
fn record_day(records: &[Transfer], record: &Transfer) -> String {
    let created = super::transfers_history::created_of(records, record.batch);
    if created > 0 {
        day_key(created)
    } else {
        String::new()
    }
}

/// 这一天能看到它吗：进行中 / 排队 / 已中断永远可见；其余按批次入队那天。
fn matches_day(records: &[Transfer], record: &Transfer, day: &str) -> bool {
    if record.state.is_active() || record.state == TransferState::Interrupted {
        return true;
    }
    if day.is_empty() || day == HISTORY_DAY_ALL {
        return true;
    }
    record_day(records, record) == day
}

/// 这一行是「读目录前整体失败 / 整个中断」的文件夹总账行吗（整批重试用它自己的目录树重来）。
pub(crate) fn is_whole_folder_ledger(records: &[Transfer], key: &str) -> bool {
    let Some(record) = records
        .iter()
        .find(|record| record_key(record) == key && is_folder_ledger(record))
    else {
        return false;
    };
    let has_children = records
        .iter()
        .any(|row| row.batch == record.batch && row.child);
    if has_children {
        return false;
    }
    matches!(
        record.state,
        TransferState::Failed | TransferState::Interrupted
    )
}

/// 主列表快照（按日期过滤）：进行中 / 排队 / 已中断排最上面，其余只看选中那天。
pub(crate) fn build_history_snapshot(records: &[Transfer], day: &str) -> (QVariantList, i32) {
    let visible: Vec<&Transfer> = ordered(records)
        .into_iter()
        .filter(|record| matches_day(records, record, day))
        .collect();
    let total = visible.len();
    let mut list = QVariantList::default();
    for record in visible.into_iter().take(MAX_ROWS) {
        list.append(QVariant::from(&record_map(record, records)));
    }
    if total > MAX_ROWS {
        list.append(QVariant::from(&notice_map(total - MAX_ROWS)));
    }
    let active = records
        .iter()
        .filter(|record| !record.child && record.state.is_active())
        .count();
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
