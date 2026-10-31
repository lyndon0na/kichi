//! 资料库三页共用的状态、刷新、文件夹打开与文件页定位。

use std::collections::HashSet;
use std::pin::Pin;
use std::time::{Duration, Instant};

use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;
use kichi_core::{EntryKind, LibraryKind};

use super::actions::run_in_background;
use super::files::{Crumb, ROOT_NAME};
use super::qobject;
use super::QVariantList;

/// 同一资料库页的自动 / 手动刷新短冷却，与分享、离线任务页保持一致。
const REFRESH_COOLDOWN: Duration = Duration::from_secs(3);

#[path = "library_view.rs"]
mod library_view;

impl qobject::Kichi {
    /// 刷新当前资料库页面。与文件目录共用 `filesLoading` 时容易混淆，因此使用
    /// 独立 loading / error 状态；请求按 epoch 作废，快速切页不会贴错集合。
    pub fn refresh_library(mut self: Pin<&mut Self>, kind: QString) {
        let Some(kind) = LibraryKind::from_key(&kind.to_string()) else {
            self.as_mut().set_message("未知的资料库页面");
            return;
        };
        let now = Instant::now();
        let session = self.backend.session.clone();
        let epoch = {
            let mut rust = self.as_mut().rust_mut();
            let current_kind = rust.library_kind.to_string();
            if !should_start_refresh(
                &current_kind,
                kind.key(),
                rust.library_loading,
                rust.library_refresh_at,
                now,
            ) {
                return;
            }
            let switching = current_kind != kind.key();
            if switching {
                rust.library_rows = QVariantList::default();
                rust.library_entries.clear();
                rust.library_loaded = false;
                // 新页面先回到顶部；同一页刷新则保留列表和当前行。
                rust.library_revision = rust.library_revision.wrapping_add(1);
            }
            rust.library_epoch = rust.library_epoch.wrapping_add(1);
            rust.library_kind = QString::from(kind.key());
            rust.library_loading = true;
            rust.library_refresh_at = Some(now);
            rust.library_error = QString::default();
            rust.library_epoch
        };
        self.as_mut().notify();

        run_in_background(
            move || session.load_library(kind),
            move |mut qichi, result| {
                if qichi.as_ref().rust().library_epoch != epoch {
                    return;
                }
                match result {
                    Ok(entries) => {
                        let rows = library_view::list_from(&entries, kind);
                        {
                            let mut rust = qichi.as_mut().rust_mut();
                            rust.library_rows = rows;
                            rust.library_entries = entries;
                            rust.library_loading = false;
                            rust.library_loaded = true;
                            rust.library_error = QString::default();
                            rust.library_refresh_at = Some(Instant::now());
                            rust.library_revision = rust.library_revision.wrapping_add(1);
                        }
                        qichi.as_mut().notify();
                    }
                    Err(err) => {
                        let text = err.to_string();
                        {
                            let mut rust = qichi.as_mut().rust_mut();
                            rust.library_loading = false;
                            rust.library_loaded = false;
                            rust.library_error = QString::from(text.as_str());
                            rust.library_refresh_at = Some(Instant::now());
                        }
                        qichi.as_mut().on_failure(err);
                    }
                }
            },
        );
    }

    /// 退出登录时丢弃旧账号的资料库与在途请求。
    pub(crate) fn reset_library(mut self: Pin<&mut Self>) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.library_epoch = rust.library_epoch.wrapping_add(1);
            rust.library_rows = QVariantList::default();
            rust.library_kind = QString::default();
            rust.library_entries.clear();
            rust.library_loading = false;
            rust.library_loaded = false;
            rust.library_error = QString::default();
            rust.library_refresh_at = None;
            rust.library_revision = rust.library_revision.wrapping_add(1);
        }
        self.as_mut().notify();
    }

    /// 星标写请求成功后更新当前资料库快照。
    ///
    /// 最近保存 / 播放历史只改星标标记；已加星标页取消星标时直接移除该行，
    /// 不额外发一轮资料库查询。
    pub(crate) fn apply_library_starred_change(
        mut self: Pin<&mut Self>,
        ids: &[String],
        starred: bool,
    ) {
        let Some(kind) = LibraryKind::from_key(&self.library_kind.to_string()) else {
            return;
        };
        let wanted: HashSet<String> = ids
            .iter()
            .map(|id| id.trim())
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
            .collect();
        if wanted.is_empty() {
            return;
        }

        {
            let mut rust = self.as_mut().rust_mut();
            let changed = update_starred_entries(&mut rust.library_entries, kind, &wanted, starred);
            if !changed {
                return;
            }
            rust.library_rows = library_view::list_from(&rust.library_entries, kind);
            rust.library_revision = rust.library_revision.wrapping_add(1);
        }
        self.as_mut().notify();
    }

    /// 从资料库打开文件夹本身。普通文件没有双击动作，也不跳到父目录定位。
    pub fn open_library_entry(mut self: Pin<&mut Self>, id: QString) {
        if self.listing {
            self.as_mut().set_message("文件列表正在加载，请稍后再打开");
            return;
        }
        let target_id = id.to_string();
        let Some(target) = self
            .library_entries
            .iter()
            .map(|entry| &entry.entry)
            .find(|entry| entry.id == target_id)
            .cloned()
        else {
            self.as_mut()
                .set_message("该条目已不在资料库中，请刷新后重试");
            return;
        };
        if target.kind != EntryKind::Folder {
            self.as_mut()
                .set_message("资料库双击只打开文件夹或可播放文件");
            return;
        }

        self.as_mut().exit_search();
        let session = self.backend.session.clone();
        let previous_crumbs = self.crumbs.clone();
        let target_name = target.name.clone();
        let epoch = {
            let mut rust = self.as_mut().rust_mut();
            rust.listing = true;
            rust.refresh_pending = false;
            rust.crumb_backup = None;
            rust.list_epoch = rust.list_epoch.wrapping_add(1);
            rust.list_epoch
        };
        self.as_mut()
            .set_message(&format!("正在打开「{target_name}」……"));

        let work_id = target_id.clone();
        run_in_background(
            move || {
                let trail = session.folder_trail(&work_id)?;
                let folder_id = trail
                    .last()
                    .map(|(id, _)| id.clone())
                    .ok_or_else(|| crate::session::Failure::Other("目录路径为空".to_owned()))?;
                let entries = session.list_dir(Some(&folder_id))?;
                Ok::<_, crate::session::Failure>((trail, entries))
            },
            move |mut qichi, result| {
                if qichi.as_ref().rust().list_epoch != epoch {
                    return;
                }
                match result {
                    Ok((trail, entries)) => {
                        {
                            let mut rust = qichi.as_mut().rust_mut();
                            rust.crumb_backup = None;
                            rust.crumbs = vec![Crumb {
                                id: String::new(),
                                name: ROOT_NAME.to_owned(),
                            }];
                            rust.crumbs
                                .extend(trail.into_iter().map(|(id, name)| Crumb { id, name }));
                        }
                        qichi.as_mut().sync_breadcrumb();
                        qichi.as_mut().clear_status_message();
                        qichi.as_mut().show_entries(entries);
                        qichi.as_mut().library_folder_ready();
                    }
                    Err(err) => {
                        {
                            let mut rust = qichi.as_mut().rust_mut();
                            rust.listing = false;
                            rust.crumbs = previous_crumbs;
                        }
                        qichi.as_mut().on_failure(err);
                    }
                }
            },
        );
    }

    /// 从资料库跳到文件页，加载目标所在父目录并在列表中选中它。
    ///
    /// 文件夹本人不进入；右键「打开文件夹」仍走 [`Self::open_library_entry`]。
    pub fn reveal_library_entry(mut self: Pin<&mut Self>, id: QString) -> bool {
        if self.listing {
            self.as_mut().set_message("文件列表正在加载，请稍后再定位");
            return false;
        }
        let target_id = id.to_string();
        let target = self
            .library_entries
            .iter()
            .map(|entry| &entry.entry)
            .find(|entry| entry.id == target_id)
            .map(|entry| (entry.kind, entry.name.clone()));
        let Some((expected_kind, target_name)) = target else {
            self.as_mut()
                .set_message("该条目已不在资料库中，请刷新后重试");
            return false;
        };

        self.as_mut().exit_search();
        let session = self.backend.session.clone();
        let previous_crumbs = self.crumbs.clone();
        let epoch = {
            let mut rust = self.as_mut().rust_mut();
            rust.listing = true;
            rust.task_folder_loading_id = QString::from(target_id.as_str());
            rust.refresh_pending = false;
            rust.list_epoch = rust.list_epoch.wrapping_add(1);
            rust.list_epoch
        };
        self.as_mut()
            .set_message(&format!("正在定位「{target_name}」……"));

        let target_for_work = target_id.clone();
        let target_for_result = target_id.clone();
        run_in_background(
            move || {
                let (_name, parent_id) = session.parent_of(&target_for_work)?;
                let (trail, list_parent) = if parent_id.trim().is_empty() {
                    (Vec::new(), None)
                } else {
                    (session.folder_trail(&parent_id)?, Some(parent_id))
                };
                let entries = session.list_dir(list_parent.as_deref())?;
                let found = entries
                    .iter()
                    .any(|entry| entry.id == target_for_work && entry.kind == expected_kind);
                if !found {
                    return Err(crate::session::Failure::Other(
                        "目标条目不在所在目录中，可能已被移动或删除".to_owned(),
                    ));
                }
                Ok((trail, entries))
            },
            move |mut qichi, result| {
                if qichi.as_ref().rust().list_epoch != epoch {
                    {
                        let mut rust = qichi.as_mut().rust_mut();
                        rust.task_folder_loading_id = QString::default();
                    }
                    qichi.as_mut().notify();
                    return;
                }
                match result {
                    Ok((trail, entries)) => {
                        {
                            let mut rust = qichi.as_mut().rust_mut();
                            rust.task_folder_loading_id = QString::default();
                            rust.crumb_backup = None;
                            rust.crumbs = vec![Crumb {
                                id: String::new(),
                                name: ROOT_NAME.to_owned(),
                            }];
                            rust.crumbs
                                .extend(trail.into_iter().map(|(id, name)| Crumb { id, name }));
                        }
                        qichi.as_mut().sync_breadcrumb();
                        qichi.as_mut().clear_status_message();
                        qichi.as_mut().show_entries(entries);
                        qichi
                            .as_mut()
                            .library_location_ready(QString::from(target_for_result.as_str()));
                    }
                    Err(err) => {
                        {
                            let mut rust = qichi.as_mut().rust_mut();
                            rust.listing = false;
                            rust.task_folder_loading_id = QString::default();
                            rust.crumbs = previous_crumbs;
                        }
                        qichi.as_mut().on_failure(err);
                    }
                }
            },
        );
        true
    }
}

fn update_starred_entries(
    entries: &mut Vec<kichi_core::LibraryEntry>,
    kind: LibraryKind,
    ids: &HashSet<String>,
    starred: bool,
) -> bool {
    if kind == LibraryKind::Starred && !starred {
        let before = entries.len();
        entries.retain(|entry| !ids.contains(&entry.entry.id));
        return entries.len() != before;
    }

    let mut changed = false;
    for entry in entries {
        if ids.contains(&entry.entry.id) && entry.entry.starred != starred {
            entry.entry.starred = starred;
            changed = true;
        }
    }
    changed
}

#[cfg(test)]
mod starred_state_tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use kichi_core::{EntryKind, FileEntry, LibraryEntry};

    fn entry(id: &str, starred: bool) -> LibraryEntry {
        LibraryEntry {
            entry: FileEntry {
                id: id.to_owned(),
                name: format!("{id}.mkv"),
                kind: EntryKind::File,
                size: 1,
                created_at: None,
                modified_at: Utc.with_ymd_and_hms(2026, 10, 31, 8, 0, 0).single(),
                parent_id: String::new(),
                md5: None,
                audit_flagged: false,
                starred,
                media: true,
                video: true,
                thumbnail_url: String::new(),
            },
            event_id: None,
            occurred_at: None,
            progress: None,
        }
    }

    fn ids(values: &[&str]) -> HashSet<String> {
        values.iter().map(|id| (*id).to_owned()).collect()
    }

    #[test]
    fn recent_and_history_entries_update_in_place() {
        let mut entries = vec![entry("a", false), entry("b", false)];
        assert!(update_starred_entries(
            &mut entries,
            LibraryKind::RecentSaved,
            &ids(&["a"]),
            true
        ));
        assert!(entries[0].entry.starred);
        assert!(!entries[1].entry.starred);
    }

    #[test]
    fn unstarring_removes_rows_from_the_starred_collection_only() {
        let mut starred = vec![entry("a", true), entry("b", true)];
        assert!(update_starred_entries(
            &mut starred,
            LibraryKind::Starred,
            &ids(&["a"]),
            false
        ));
        assert_eq!(starred.len(), 1);
        assert_eq!(starred[0].entry.id, "b");

        let mut history = vec![entry("a", true), entry("b", true)];
        assert!(update_starred_entries(
            &mut history,
            LibraryKind::PlaybackHistory,
            &ids(&["a"]),
            false
        ));
        assert_eq!(history.len(), 2);
        assert!(!history[0].entry.starred);
    }

    #[test]
    fn no_matching_entry_is_reported_as_unchanged() {
        let mut entries = vec![entry("a", false)];
        assert!(!update_starred_entries(
            &mut entries,
            LibraryKind::RecentSaved,
            &ids(&["missing"]),
            true
        ));
    }
}

/// 同一页才受冷却和单飞限制；切到另一个资料库集合必须立即换数据。
fn should_start_refresh(
    current_kind: &str,
    wanted_kind: &str,
    loading: bool,
    last_refresh: Option<Instant>,
    now: Instant,
) -> bool {
    if current_kind != wanted_kind {
        return true;
    }
    !loading
        && last_refresh.is_none_or(|last| now.saturating_duration_since(last) >= REFRESH_COOLDOWN)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switching_library_kind_ignores_cooldown_and_inflight_guard() {
        let now = Instant::now();
        assert!(should_start_refresh(
            "recent-saved",
            "starred",
            true,
            Some(now),
            now
        ));
    }

    #[test]
    fn same_library_kind_coalesces_inflight_and_cooldown() {
        let now = Instant::now();
        assert!(!should_start_refresh("starred", "starred", true, None, now));
        assert!(!should_start_refresh(
            "starred",
            "starred",
            false,
            Some(now - Duration::from_millis(2999)),
            now
        ));
        assert!(should_start_refresh(
            "starred",
            "starred",
            false,
            Some(now - REFRESH_COOLDOWN),
            now
        ));
    }
}
