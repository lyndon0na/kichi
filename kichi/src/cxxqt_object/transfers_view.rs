//! 传输记录的 QML 视图映射：一条记录 → 一行 `QVariantMap`，以及排序 / 快照。
//!
//! 从 `transfers.rs` 拆出来守行数上限：那边管状态机与记录表，这里只管
//! 「字段叫什么、什么时候显示什么、按什么顺序交给 QML」。角色名不能叫 `state`
//! （`Item` 自带同名属性，见 `docs/pitfalls.md` 第 12 条），QML 侧统一映射成 `phase`。

use cxx_qt_lib::{QMap, QMapPair_QString_QVariant, QString, QVariant};

use super::actions::format_size;
use super::transfers::{record_key, Transfer, TransferState};
use super::QVariantList;

/// 一次最多交给 QML 的行数（大文件夹下载时避免整列表反复重建拖慢界面）。
const MAX_ROWS: usize = 500;

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

pub(crate) fn record_map(record: &Transfer) -> QVariantMap {
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
    let name = record.display_name();
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
    map
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

/// 超过显示上限时追加的提示行（ListView 里最后一行）。
///
/// **必须给 `key`**：QML 的差异算法靠它识别行；缺了会让 ListModel 报
/// `<Unknown File>: key is undefined`（2026-10-27 实测），提示行也会在每次
/// 刷新时被当成新行重复插入。
pub(crate) fn notice_map(hidden: usize) -> QVariantMap {
    let mut map = QVariantMap::default();
    insert(&mut map, "key", text("notice"));
    insert(
        &mut map,
        "name",
        text(&format!(
            "还有 {hidden} 条记录未显示（页面只显示前 500 条）"
        )),
    );
    insert(&mut map, "sourceId", text(""));
    insert(&mut map, "state", text("notice"));
    insert(&mut map, "stateText", text(""));
    insert(&mut map, "running", QVariant::from(&false));
    insert(&mut map, "progress", QVariant::from(&0));
    insert(&mut map, "progressText", text(""));
    insert(&mut map, "sizeText", text(""));
    insert(&mut map, "detail", text(""));
    insert(&mut map, "path", text(""));
    map
}

/// 排序：进行中在前（批次内按顺序），已结束在后（新 → 旧）。
pub(crate) fn ordered(records: &[Transfer]) -> Vec<&Transfer> {
    let mut active: Vec<&Transfer> = records
        .iter()
        .filter(|record| record.state.is_active())
        .collect();
    let mut finished: Vec<&Transfer> = records
        .iter()
        .filter(|record| record.state.is_finished())
        .collect();
    active.sort_by_key(|record| (record.batch, record.index));
    finished.sort_by(|a, b| (b.batch, b.index).cmp(&(a.batch, a.index)));
    active.extend(finished);
    active
}

/// 记录表 → QML 模型 + 进行中条数（排队 + 准备中 + 下载中）。
pub(crate) fn build_snapshot(records: &[Transfer]) -> (QVariantList, i32) {
    let total = records.len();
    let active = records
        .iter()
        .filter(|record| record.state.is_active())
        .count();
    let mut list = QVariantList::default();
    for record in ordered(records).into_iter().take(MAX_ROWS) {
        list.append(QVariant::from(&record_map(record)));
    }
    if total > MAX_ROWS {
        list.append(QVariant::from(&notice_map(total - MAX_ROWS)));
    }
    let active = i32::try_from(active).unwrap_or(i32::MAX);
    (list, active)
}
