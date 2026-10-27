//! 传输记录的 QML 视图映射：一条记录 → 一行 `QVariantMap`。
//!
//! 从 `transfers.rs` 拆出来守行数上限：那边管状态机与排序，这里只管
//! 「字段叫什么、什么时候显示什么」。角色名不能叫 `state`（`Item` 自带同名
//! 属性，见 `docs/pitfalls.md` 第 12 条），QML 侧统一映射成 `phase`。

use cxx_qt_lib::{QMap, QMapPair_QString_QVariant, QString, QVariant};

use super::actions::format_size;
use super::transfers::{Transfer, TransferState};

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

/// 记录的稳定标识（QML 用它把实时进度贴到对应的行上）。
pub(crate) fn record_key(record: &Transfer) -> String {
    format!("{}:{}", record.batch, record.index)
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
    // 准备中的说明行直接由状态决定：读目录期间没有别的信息可写
    let detail = if record.state == TransferState::Preparing {
        "正在读取目录结构…".to_owned()
    } else {
        record.detail.clone()
    };

    let mut map = QVariantMap::default();
    insert(&mut map, "key", text(&record_key(record)));
    insert(&mut map, "name", text(&record.name));
    insert(&mut map, "sourceId", text(&record.source_id));
    insert(&mut map, "state", text(record.state.name()));
    insert(&mut map, "stateText", text(record.state.text()));
    insert(&mut map, "running", QVariant::from(&running));
    insert(&mut map, "progress", QVariant::from(&progress));
    insert(&mut map, "progressText", text(&progress_text));
    insert(&mut map, "sizeText", text(&size_text));
    insert(&mut map, "detail", text(&detail));
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
    map
}
