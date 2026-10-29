//! 回收站领域对象到 QML 表格模型的转换。

use cxx_qt_lib::{QMap, QMapPair_QString_QVariant, QString, QVariant};
use kichi_core::TrashEntry;

use super::QVariantList;

type QVariantMap = QMap<QMapPair_QString_QVariant>;

pub(super) fn sort_trash_entries(entries: &mut [TrashEntry]) {
    entries.sort_by(|a, b| {
        b.deleted_at
            .cmp(&a.deleted_at)
            .then_with(|| {
                a.entry
                    .name
                    .to_lowercase()
                    .cmp(&b.entry.name.to_lowercase())
            })
            .then_with(|| a.entry.id.cmp(&b.entry.id))
    });
}

pub(super) fn trash_list_from(entries: &[TrashEntry]) -> QVariantList {
    let mut files = QVariantList::default();
    for entry in entries {
        files.append(QVariant::from(&trash_entry_map(entry)));
    }
    files
}

fn trash_entry_map(entry: &TrashEntry) -> QVariantMap {
    let mut map = super::super::files::entry_map(&entry.entry);
    let deleted = entry
        .deleted_at
        .map(|time| {
            time.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "—".to_owned());
    map.insert_clone(
        &QString::from("deletedText"),
        &QVariant::from(&QString::from(deleted)),
    );
    map.insert_clone(
        &QString::from("parentId"),
        &QVariant::from(&QString::from(entry.parent_id.as_str())),
    );
    map
}
