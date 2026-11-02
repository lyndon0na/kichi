//! 回收站写操作的本地乐观状态。
//!
//! 确认删除后先把条目的位置留给回滚快照，再发布不含它们的列表；
//! 分批失败时只恢复失败批与尚未发送的条目。整库清空失败则恢复完整快照。

use std::collections::{HashSet, VecDeque};
use std::pin::Pin;
use std::time::Instant;

use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;
use kichi_core::TrashEntry;

use super::super::{qobject, KichiRust, QVariantList};
use super::trash_actions::TrashActionKind;
use super::trash_view::{sort_trash_entries, trash_list_from};

impl qobject::Kichi {
    /// 本地先移除待彻底删除的条目，返回请求失败时用于恢复的快照。
    pub(super) fn remove_trash_entries_locally(
        mut self: Pin<&mut Self>,
        ids: &[String],
    ) -> Vec<TrashEntry> {
        let removed = {
            let mut rust = self.as_mut().rust_mut();
            cancel_trash_refresh(&mut rust);
            let removed = take_removed_entries(&mut rust.trash_list, ids);
            if !removed.is_empty() {
                rust.trash_rows = trash_list_from(&rust.trash_list);
                rust.trash_revision = rust.trash_revision.wrapping_add(1);
            }
            rust.trash_loaded = true;
            removed
        };
        if !removed.is_empty() {
            self.as_mut().notify();
        }
        removed
    }

    /// 本地先清空回收站，保留整份快照供请求失败时恢复。
    pub(super) fn clear_trash_locally(mut self: Pin<&mut Self>) -> Vec<TrashEntry> {
        let removed = {
            let mut rust = self.as_mut().rust_mut();
            cancel_trash_refresh(&mut rust);
            let removed = std::mem::take(&mut rust.trash_list);
            rust.trash_rows = QVariantList::default();
            rust.trash_revision = rust.trash_revision.wrapping_add(1);
            rust.trash_loaded = true;
            removed
        };
        self.as_mut().notify();
        removed
    }

    /// 从乐观快照里取出本批失败及尚未发送的条目，已完成项保持删除。
    pub(super) fn take_entries_to_restore(
        mut self: Pin<&mut Self>,
        failed_ids: &[String],
    ) -> Vec<TrashEntry> {
        let mut rust = self.as_mut().rust_mut();
        let Some(state) = rust.trash_action.as_mut() else {
            return Vec::new();
        };
        entries_to_restore(state.kind, &mut state.removed, &state.remaining, failed_ids)
    }

    pub(super) fn publish_restored_entries(mut self: Pin<&mut Self>, restored: Vec<TrashEntry>) {
        if restored.is_empty() {
            return;
        }
        {
            let mut rust = self.as_mut().rust_mut();
            rust.trash_list.extend(restored);
            sort_trash_entries(&mut rust.trash_list);
            rust.trash_rows = trash_list_from(&rust.trash_list);
            rust.trash_revision = rust.trash_revision.wrapping_add(1);
            rust.trash_loaded = true;
            rust.trash_refresh_at = Some(Instant::now());
            rust.trash_error = QString::default();
        }
        self.as_mut().notify();
    }
}

fn cancel_trash_refresh(rust: &mut KichiRust) {
    rust.trash_epoch = rust.trash_epoch.wrapping_add(1);
    rust.trash_loading = false;
    rust.trash_request_running = false;
    rust.trash_refresh_at = Some(Instant::now());
    rust.trash_error = QString::default();
}

pub(super) fn take_removed_entries(
    entries: &mut Vec<TrashEntry>,
    ids: &[String],
) -> Vec<TrashEntry> {
    let wanted: HashSet<&str> = ids.iter().map(String::as_str).collect();
    if wanted.is_empty() {
        return Vec::new();
    }
    let mut kept = Vec::with_capacity(entries.len());
    let mut removed = Vec::new();
    for entry in std::mem::take(entries) {
        if wanted.contains(entry.entry.id.as_str()) {
            removed.push(entry);
        } else {
            kept.push(entry);
        }
    }
    *entries = kept;
    removed
}

pub(super) fn entries_to_restore(
    kind: TrashActionKind,
    removed: &mut Vec<TrashEntry>,
    remaining: &VecDeque<String>,
    failed_ids: &[String],
) -> Vec<TrashEntry> {
    match kind {
        TrashActionKind::Restore => Vec::new(),
        TrashActionKind::Clear => std::mem::take(removed),
        TrashActionKind::Delete => {
            let mut pending: HashSet<String> = remaining.iter().cloned().collect();
            pending.extend(failed_ids.iter().cloned());
            removed
                .drain(..)
                .filter(|entry| pending.contains(&entry.entry.id))
                .collect()
        }
    }
}
