//! 文件页写操作的本地先行动作。
//!
//! 请求发出前先改当前目录快照，界面立即看到结果；云端失败时用操作前快照
//! 原位回滚。新建文件夹成功时官方不回 id，因此临时行会保留到下一次目录
//! 校准发现真实条目为止，期间不可进入。

use std::collections::HashSet;
use std::pin::Pin;

use chrono::Utc;
use cxx_qt::CxxQtType;
use kichi_core::{EntryKind, FileEntry};

use super::files::{list_from_with_pending, sort_entries};
use super::{qobject, KichiRust};
use crate::session::SearchHit;

const TEMP_FOLDER_PREFIX: &str = "kichi-pending-folder-";

pub(crate) struct PendingCreate {
    entry: FileEntry,
    known_ids: HashSet<String>,
}

impl PendingCreate {
    fn matching_new_id(
        &self,
        entries: &[FileEntry],
        already_matched: &HashSet<String>,
    ) -> Option<String> {
        entries
            .iter()
            .find(|entry| {
                entry.kind == EntryKind::Folder
                    && entry.name == self.entry.name
                    && entry.parent_id == self.entry.parent_id
                    && !entry.id.starts_with(TEMP_FOLDER_PREFIX)
                    && !self.known_ids.contains(&entry.id)
                    && !already_matched.contains(&entry.id)
            })
            .map(|entry| entry.id.clone())
    }
}

pub(crate) enum FileMutation {
    Rename {
        dir_id: String,
        id: String,
        before_name: String,
    },
    Create {
        dir_id: String,
        pending: Box<PendingCreate>,
    },
    Remove {
        dir_id: String,
        removed: Vec<(usize, FileEntry)>,
        search_before: Vec<SearchHit>,
    },
}

pub(crate) struct StarredSnapshot {
    dir_id: String,
    changes: Vec<(String, bool)>,
}

pub(crate) fn pending_folder_id(action_id: u64) -> String {
    format!("{TEMP_FOLDER_PREFIX}{action_id}")
}

impl qobject::Kichi {
    pub(crate) fn optimistic_rename(
        mut self: Pin<&mut Self>,
        action_id: u64,
        id: &str,
        name: &str,
    ) -> bool {
        let id = id.trim();
        if id.is_empty() {
            return false;
        }
        let before_name = self
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| entry.name.clone())
            .or_else(|| self.search_index.get(id).map(|hit| hit.entry.name.clone()));
        let Some(before_name) = before_name else {
            return false;
        };
        let index = self.entries.iter().position(|entry| entry.id == id);
        let has_search_hit = self.search_active && self.search_index.contains_key(id);
        if index.is_none() && !has_search_hit {
            return false;
        }
        self.as_mut().cancel_files_refresh_for_mutation();
        let mut next = self.entries.clone();
        if let Some(index) = index {
            next[index].name = name.to_owned();
        }
        {
            let mut rust = self.as_mut().rust_mut();
            if let Some(hit) = rust.search_index.get_mut(id) {
                hit.entry.name = name.to_owned();
            }
            let dir_id = rust.current_dir_id.to_string();
            rust.file_mutations.insert(
                action_id,
                FileMutation::Rename {
                    dir_id,
                    id: id.to_owned(),
                    before_name,
                },
            );
        }
        self.as_mut().publish_optimistic_entries(next);
        true
    }

    pub(crate) fn optimistic_create_folder(
        mut self: Pin<&mut Self>,
        action_id: u64,
        name: &str,
        temp_id: &str,
    ) -> bool {
        self.as_mut().cancel_files_refresh_for_mutation();
        let before_entries = self.entries.clone();
        let parent_id = self.current_dir_id.to_string();
        let now = Utc::now();
        let entry = FileEntry {
            id: temp_id.to_owned(),
            name: name.to_owned(),
            kind: EntryKind::Folder,
            size: 0,
            created_at: Some(now),
            modified_at: Some(now),
            parent_id,
            md5: None,
            audit_flagged: false,
            starred: false,
            media: false,
            video: false,
            thumbnail_url: String::new(),
        };
        let known_ids = before_entries.iter().map(|item| item.id.clone()).collect();
        let mut next = before_entries.clone();
        next.push(entry.clone());
        {
            let mut rust = self.as_mut().rust_mut();
            rust.pending_entry_ids.insert(temp_id.to_owned());
            let dir_id = rust.current_dir_id.to_string();
            rust.file_mutations.insert(
                action_id,
                FileMutation::Create {
                    dir_id,
                    pending: Box::new(PendingCreate { entry, known_ids }),
                },
            );
        }
        self.as_mut().publish_optimistic_entries(next);
        true
    }

    pub(crate) fn optimistic_remove_entries(
        mut self: Pin<&mut Self>,
        action_id: u64,
        ids: &[String],
    ) -> bool {
        let wanted: HashSet<String> = ids
            .iter()
            .map(|id| id.trim())
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
            .collect();
        if wanted.is_empty() {
            return false;
        }
        self.as_mut().cancel_files_refresh_for_mutation();
        let before_entries = self.entries.clone();
        let removed: Vec<(usize, FileEntry)> = before_entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| wanted.contains(&entry.id))
            .map(|(index, entry)| (index, entry.clone()))
            .collect();
        let search_before = self.search_hits_for(&wanted);
        let mut next = before_entries.clone();
        next.retain(|entry| !wanted.contains(&entry.id));
        if removed.is_empty() && search_before.is_empty() {
            return false;
        }
        {
            let mut rust = self.as_mut().rust_mut();
            rust.search_index.retain(|id, _| !wanted.contains(id));
            let dir_id = rust.current_dir_id.to_string();
            rust.file_mutations.insert(
                action_id,
                FileMutation::Remove {
                    dir_id,
                    removed,
                    search_before,
                },
            );
        }
        self.as_mut().publish_optimistic_entries(next);
        true
    }

    /// 成功：清掉这个动作的回滚快照。新建文件夹转成“等待真实 id”的临时行。
    pub(crate) fn commit_file_mutation(mut self: Pin<&mut Self>, action_id: u64) {
        let mutation = self.as_mut().rust_mut().file_mutations.remove(&action_id);
        self.as_mut().mark_library_caches_stale(None);
        if let Some(FileMutation::Create { pending, .. }) = mutation {
            self.as_mut().rust_mut().unresolved_create.push(*pending);
        }
    }

    /// 失败：只撤回这个动作碰过的条目，保留同一目录里其它并发动作的结果。
    pub(crate) fn rollback_file_mutation(mut self: Pin<&mut Self>, action_id: u64) {
        let Some(mutation) = self.as_mut().rust_mut().file_mutations.remove(&action_id) else {
            return;
        };
        let current_dir = self.current_dir_id.to_string();
        if let FileMutation::Create { pending, .. } = &mutation {
            self.as_mut()
                .rust_mut()
                .pending_entry_ids
                .remove(&pending.entry.id);
        }
        match mutation {
            FileMutation::Rename {
                dir_id,
                id,
                before_name,
            } => {
                let same_dir = current_dir == dir_id;
                {
                    let mut rust = self.as_mut().rust_mut();
                    if same_dir {
                        if let Some(entry) = rust.entries.iter_mut().find(|entry| entry.id == id) {
                            entry.name = before_name.clone();
                        }
                    }
                    if let Some(hit) = rust.search_index.get_mut(&id) {
                        hit.entry.name = before_name.clone();
                    }
                    rust.files_cache.restore_name(&dir_id, &id, &before_name);
                }
                if same_dir {
                    let entries = self.entries.clone();
                    self.as_mut().publish_optimistic_entries(entries);
                } else {
                    self.as_mut().rebuild_search_view_from_state();
                }
            }
            FileMutation::Create { dir_id, pending } => {
                let same_dir = current_dir == dir_id;
                {
                    let mut rust = self.as_mut().rust_mut();
                    if same_dir {
                        rust.entries.retain(|entry| entry.id != pending.entry.id);
                    }
                    rust.files_cache.remove_entry(&dir_id, &pending.entry.id);
                }
                if same_dir {
                    let entries = self.entries.clone();
                    self.as_mut().publish_optimistic_entries(entries);
                } else {
                    self.as_mut().rebuild_search_view_from_state();
                }
            }
            FileMutation::Remove {
                dir_id,
                removed,
                search_before,
            } => {
                let same_dir = current_dir == dir_id;
                {
                    let mut rust = self.as_mut().rust_mut();
                    for hit in search_before {
                        rust.search_index.insert(hit.id().to_owned(), hit);
                    }
                    if same_dir {
                        restore_removed_entries(&mut rust.entries, &removed);
                    }
                    rust.files_cache.restore_entries(&dir_id, &removed);
                }
                if same_dir {
                    let entries = self.entries.clone();
                    self.as_mut().publish_optimistic_entries(entries);
                } else {
                    self.as_mut().rebuild_search_view_from_state();
                }
            }
        }
    }

    pub(crate) fn optimistic_starred(
        mut self: Pin<&mut Self>,
        ids: &[String],
        starred: bool,
    ) -> Option<StarredSnapshot> {
        let wanted: HashSet<String> = ids
            .iter()
            .map(|id| id.trim())
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
            .collect();
        if wanted.is_empty() {
            return None;
        }
        let mut before = Vec::new();
        for id in &wanted {
            if let Some(entry) = self.entries.iter().find(|entry| entry.id == *id) {
                before.push((id.clone(), entry.starred));
            } else if let Some(hit) = self.search_index.get(id) {
                before.push((id.clone(), hit.entry.starred));
            }
        }
        if before.is_empty() {
            return None;
        }
        self.as_mut().cancel_files_refresh_for_mutation();
        for entry in &mut self.as_mut().rust_mut().entries {
            if wanted.contains(&entry.id) {
                entry.starred = starred;
            }
        }
        {
            let mut rust = self.as_mut().rust_mut();
            for hit in rust.search_index.values_mut() {
                if wanted.contains(&hit.entry.id) {
                    hit.entry.starred = starred;
                }
            }
        }
        let dir_id = self.current_dir_id.to_string();
        let entries = self.entries.clone();
        self.as_mut().publish_optimistic_entries(entries);
        Some(StarredSnapshot {
            dir_id,
            changes: before,
        })
    }

    pub(crate) fn restore_starred(mut self: Pin<&mut Self>, snapshot: StarredSnapshot) {
        if snapshot.changes.is_empty() {
            return;
        }
        let same_dir = self.current_dir_id.to_string() == snapshot.dir_id;
        {
            let mut rust = self.as_mut().rust_mut();
            if same_dir {
                for (id, starred) in &snapshot.changes {
                    if let Some(entry) = rust.entries.iter_mut().find(|entry| entry.id == *id) {
                        entry.starred = *starred;
                    }
                }
            }
            rust.files_cache
                .restore_starred(&snapshot.dir_id, &snapshot.changes);
            for (id, starred) in &snapshot.changes {
                if let Some(hit) = rust.search_index.get_mut(id) {
                    hit.entry.starred = *starred;
                }
            }
        }
        if same_dir {
            let entries = self.entries.clone();
            self.as_mut().publish_optimistic_entries(entries);
        } else {
            self.as_mut().rebuild_search_view_from_state();
        }
    }

    fn publish_optimistic_entries(mut self: Pin<&mut Self>, mut entries: Vec<FileEntry>) {
        let dir_id = self.current_dir_id.to_string();
        {
            let mut rust = self.as_mut().rust_mut();
            sort_entries(&mut entries, rust.sort_field, rust.sort_ascending);
            rust.files = list_from_with_pending(&entries, &rust.pending_entry_ids);
            rust.files_cache.replace_entries(dir_id, &entries);
            rust.entries = entries;
            rust.files_revision = rust.files_revision.wrapping_add(1);
            rust.files_stale = false;
        }
        self.as_mut().notify();
        self.as_mut().rebuild_search_view_from_state();
    }

    fn search_hits_for(&self, ids: &HashSet<String>) -> Vec<SearchHit> {
        if !self.search_active {
            return Vec::new();
        }
        ids.iter()
            .filter_map(|id| self.search_index.get(id).cloned())
            .collect()
    }
}

/// 官方目录已经返回时，把尚未换到真实 id 的临时文件夹合回去。
///
/// 只认本次新建后出现的新 id，避免同目录里原本就叫这个名字的文件夹
/// 被误当成刚创建成功。
pub(crate) fn merge_unresolved_create(
    rust: &mut KichiRust,
    entries: Vec<FileEntry>,
) -> (Vec<FileEntry>, Vec<String>) {
    // 用户已经切到别的目录时，临时行不能跟着这一次目录结果塞进去。
    // 保留待确认状态，等回到原父目录后的下一次读取再换真实 id。
    let current_dir = rust.current_dir_id.to_string();
    let pending_creates = std::mem::take(&mut rust.unresolved_create);
    let merged = merge_pending_creates(&current_dir, pending_creates, entries);
    for temp_id in &merged.added_temp_ids {
        rust.pending_entry_ids.insert(temp_id.clone());
    }
    for temp_id in &merged.resolved_temp_ids {
        rust.pending_entry_ids.remove(temp_id);
    }
    rust.unresolved_create = merged.unresolved;
    (merged.entries, merged.resolved_real_ids)
}

struct MergedCreates {
    entries: Vec<FileEntry>,
    unresolved: Vec<PendingCreate>,
    added_temp_ids: Vec<String>,
    resolved_temp_ids: Vec<String>,
    resolved_real_ids: Vec<String>,
}

fn merge_pending_creates(
    current_dir: &str,
    pending_creates: Vec<PendingCreate>,
    mut entries: Vec<FileEntry>,
) -> MergedCreates {
    let mut unresolved = Vec::with_capacity(pending_creates.len());
    let mut added_temp_ids = Vec::new();
    let mut resolved_temp_ids = Vec::new();
    let mut resolved_real_ids = Vec::new();
    let mut matched_ids = HashSet::new();
    for pending in pending_creates {
        if pending.entry.parent_id != current_dir {
            unresolved.push(pending);
            continue;
        }
        if let Some(real_id) = pending.matching_new_id(&entries, &matched_ids) {
            matched_ids.insert(real_id.clone());
            resolved_temp_ids.push(pending.entry.id);
            resolved_real_ids.push(real_id);
            continue;
        }
        let temp_id = pending.entry.id.clone();
        entries.push(pending.entry.clone());
        added_temp_ids.push(temp_id);
        unresolved.push(pending);
    }
    MergedCreates {
        entries,
        unresolved,
        added_temp_ids,
        resolved_temp_ids,
        resolved_real_ids,
    }
}

fn restore_removed_entries(entries: &mut Vec<FileEntry>, removed: &[(usize, FileEntry)]) {
    let mut restored: Vec<(usize, FileEntry)> = removed
        .iter()
        .filter(|(_, removed)| !entries.iter().any(|entry| entry.id == removed.id))
        .cloned()
        .collect();
    restored.sort_by_key(|(index, _)| *index);
    for (index, entry) in restored {
        let at = index.min(entries.len());
        entries.insert(at, entry);
    }
}

pub(crate) fn is_unconfirmed_create(rust: &KichiRust, id: &str) -> bool {
    rust.pending_entry_ids.contains(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn folder(id: &str, name: &str, parent_id: &str) -> FileEntry {
        FileEntry {
            id: id.to_owned(),
            name: name.to_owned(),
            kind: EntryKind::Folder,
            size: 0,
            created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).single(),
            modified_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).single(),
            parent_id: parent_id.to_owned(),
            md5: None,
            audit_flagged: false,
            starred: false,
            media: false,
            video: false,
            thumbnail_url: String::new(),
        }
    }

    #[test]
    fn pending_create_only_resolves_to_a_new_matching_folder() {
        let old = folder("old", "新建文件夹", "parent");
        let pending = PendingCreate {
            entry: folder("temp", "新建文件夹", "parent"),
            known_ids: HashSet::from([old.id.clone()]),
        };
        assert!(pending
            .matching_new_id(std::slice::from_ref(&old), &HashSet::new())
            .is_none());
        let real = folder("real", "新建文件夹", "parent");
        assert!(pending
            .matching_new_id(&[old, real], &HashSet::new())
            .is_some());
    }

    #[test]
    fn multiple_pending_creates_consume_distinct_real_ids() {
        let old = folder("old", "新建文件夹", "parent");
        let pending = PendingCreate {
            entry: folder("temp", "新建文件夹", "parent"),
            known_ids: HashSet::from([old.id.clone()]),
        };
        let real_a = folder("real-a", "新建文件夹", "parent");
        let real_b = folder("real-b", "新建文件夹", "parent");
        let entries = [old, real_a, real_b];
        let mut matched = HashSet::new();

        let first = pending
            .matching_new_id(&entries, &matched)
            .expect("第一个真实目录");
        matched.insert(first.clone());
        let second = pending
            .matching_new_id(&entries, &matched)
            .expect("第二个真实目录");

        assert_ne!(first, second);
    }

    #[test]
    fn pending_row_itself_is_never_mistaken_for_a_real_id() {
        let pending = PendingCreate {
            entry: folder("kichi-pending-folder-7", "新建文件夹", "parent"),
            known_ids: HashSet::from(["old".to_owned()]),
        };
        let same_pending_row = pending.entry.clone();

        assert!(pending
            .matching_new_id(&[same_pending_row], &HashSet::new())
            .is_none());
    }

    #[test]
    fn merge_reports_the_real_id_only_after_the_temp_row_is_resolved() {
        let temp_id = "kichi-pending-folder-1".to_owned();
        let pending = PendingCreate {
            entry: folder(&temp_id, "新建文件夹", "parent"),
            known_ids: HashSet::from(["old".to_owned()]),
        };

        let pending_merge = merge_pending_creates(
            "parent",
            vec![pending],
            vec![folder("old", "新建文件夹", "parent")],
        );
        assert!(pending_merge.resolved_real_ids.is_empty());
        assert!(pending_merge
            .entries
            .iter()
            .any(|entry| entry.id == temp_id));

        let resolved = merge_pending_creates(
            "parent",
            pending_merge.unresolved,
            vec![
                folder("old", "新建文件夹", "parent"),
                folder("real", "新建文件夹", "parent"),
            ],
        );
        assert_eq!(resolved.resolved_real_ids, vec!["real"]);
        assert_eq!(resolved.resolved_temp_ids, vec![temp_id]);
        assert!(resolved.unresolved.is_empty());
        assert!(!resolved
            .entries
            .iter()
            .any(|entry| entry.id.starts_with(TEMP_FOLDER_PREFIX)));
    }
}
