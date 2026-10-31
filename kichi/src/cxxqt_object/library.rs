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
/// 页面重新可见时，资料库缓存超过这个时间才在后台校准。
const LIBRARY_REFRESH_TTL: Duration = Duration::from_secs(30);

#[path = "library_view.rs"]
mod library_view;

/// 一个资料库集合的最近一次快照。
///
/// 三页共用桥接属性，但底层数据不能共用一份：切到另一页时先把对应快照放回
/// 属性，再静默校准；切回上一页仍能看到上次内容和滚动位置。
#[derive(Clone)]
pub(crate) struct LibraryCache {
    pub(crate) rows: QVariantList,
    pub(crate) entries: Vec<kichi_core::LibraryEntry>,
    pub(crate) loaded: bool,
    pub(crate) error: String,
    pub(crate) refresh_at: Option<Instant>,
}

impl LibraryCache {
    fn is_fresh(&self, now: Instant) -> bool {
        self.loaded
            && self
                .refresh_at
                .is_some_and(|at| now.saturating_duration_since(at) < LIBRARY_REFRESH_TTL)
    }
}

/// 资料库星标的乐观快照；失败时只恢复发起操作的那个集合。
pub(crate) struct LibraryStarSnapshot {
    pub(crate) kind: LibraryKind,
    entries: Vec<kichi_core::LibraryEntry>,
    rows: QVariantList,
    refresh_at: Option<Instant>,
    cache: Option<LibraryCache>,
}

impl qobject::Kichi {
    /// 手动刷新资料库页面：绕过切页 TTL，但仍合并短时间内的重复点击 / 在途请求。
    pub fn refresh_library(self: Pin<&mut Self>, kind: QString) {
        self.refresh_library_inner(kind, false, true);
    }

    /// 页面重新可见：先恢复该集合的缓存，缓存新鲜时不再发请求。
    pub fn ensure_library_fresh(self: Pin<&mut Self>, kind: QString) {
        self.refresh_library_inner(kind, true, false);
    }

    /// 刷新当前资料库页面。与文件目录共用 `filesLoading` 时容易混淆，因此使用
    /// 独立 loading / error 状态；请求按 epoch 作废，快速切页不会贴错集合。
    fn refresh_library_inner(
        mut self: Pin<&mut Self>,
        kind: QString,
        cache_first: bool,
        manual: bool,
    ) {
        let Some(kind) = LibraryKind::from_key(&kind.to_string()) else {
            self.as_mut().set_message("未知的资料库页面");
            return;
        };
        let now = Instant::now();
        let session = self.backend.session.clone();
        let (epoch, restored, had_cache, manual_visible) = {
            let mut rust = self.as_mut().rust_mut();
            let current_kind = rust.library_kind.to_string();
            let switching = current_kind != kind.key();
            let mut restored = false;
            if switching {
                if rust.library_cache.contains_key(kind.key()) {
                    restored = restore_library_cache(&mut rust, kind);
                } else {
                    clear_library_view(&mut rust, kind);
                }
            }

            let had_cache = rust.library_loaded || !rust.library_entries.is_empty();
            let can_request = !rust.file_action_running;
            let should_request = can_request
                && if switching {
                    // 切集合时先恢复目标缓存；只有“切页”模式允许新鲜缓存直接复用。
                    !(cache_first && restored && library_cache_is_fresh(&rust, kind, now))
                } else if cache_first {
                    !rust.library_request_running && !library_cache_is_fresh(&rust, kind, now)
                } else {
                    !rust.library_request_running
                        && rust.library_refresh_at.is_none_or(|last| {
                            now.saturating_duration_since(last) >= REFRESH_COOLDOWN
                        })
                };

            if should_request {
                let visible = manual || !had_cache;
                (
                    Some(begin_library_refresh(&mut rust, kind, now, visible)),
                    restored,
                    had_cache,
                    false,
                )
            } else {
                let mut manual_visible = false;
                if switching || restored {
                    // 立即作废上一集合的在途结果；缓存本身已在上面恢复。
                    rust.library_epoch = rust.library_epoch.wrapping_add(1);
                    rust.library_request_running = false;
                } else if manual
                    && had_cache
                    && rust.library_request_running
                    && !rust.library_loading
                {
                    // 已有一轮自动校准在途时，点刷新直接并入它，但立刻给出手动反馈。
                    rust.library_loading = true;
                    manual_visible = true;
                }
                if !can_request && manual && rust.library_kind.to_string() == kind.key() {
                    // 乐观写入尚未提交时不能拉服务端旧快照；用户主动点的刷新
                    // 排队到写操作收尾后执行，刷新按钮只显示这次真实请求的状态。
                    rust.library_refresh_pending = Some(kind);
                    if !rust.library_loading {
                        rust.library_loading = true;
                        manual_visible = true;
                    }
                }
                (None, restored, had_cache, manual_visible)
            }
        };
        if restored || epoch.is_some() || manual_visible {
            self.as_mut().notify();
        }
        let Some(epoch) = epoch else {
            return;
        };

        run_in_background(
            move || session.load_library(kind),
            move |mut qichi, result| {
                if qichi.as_ref().rust().library_epoch != epoch {
                    return;
                }
                match result {
                    Ok(entries) => {
                        let rows = library_view::list_from(&entries, kind);
                        let refresh_at = Instant::now();
                        let mut notify = false;
                        {
                            let mut rust = qichi.as_mut().rust_mut();
                            rust.library_cache.insert(
                                kind.key(),
                                LibraryCache {
                                    rows: rows.clone(),
                                    entries: entries.clone(),
                                    loaded: true,
                                    error: String::new(),
                                    refresh_at: Some(refresh_at),
                                },
                            );
                            if rust.library_kind.to_string() == kind.key() {
                                let changed = rust.library_entries != entries;
                                let visible = rust.library_loading;
                                if changed {
                                    rust.library_rows = rows;
                                    rust.library_revision = rust.library_revision.wrapping_add(1);
                                }
                                rust.library_entries = entries;
                                rust.library_loading = false;
                                rust.library_request_running = false;
                                rust.library_loaded = true;
                                rust.library_error = QString::default();
                                rust.library_refresh_at = Some(refresh_at);
                                notify = changed || visible;
                            } else {
                                rust.library_request_running = false;
                            }
                        }
                        if notify {
                            qichi.as_mut().notify();
                        }
                    }
                    Err(err) => {
                        let text = err.to_string();
                        let refresh_at = Instant::now();
                        {
                            let mut rust = qichi.as_mut().rust_mut();
                            if let Some(cache) = rust.library_cache.get_mut(kind.key()) {
                                if !had_cache {
                                    cache.error = text.clone();
                                }
                                cache.refresh_at = Some(refresh_at);
                            }
                            if rust.library_kind.to_string() == kind.key() {
                                rust.library_loading = false;
                                rust.library_request_running = false;
                                if !had_cache {
                                    rust.library_error = QString::from(text.as_str());
                                }
                                rust.library_refresh_at = Some(refresh_at);
                            }
                        }
                        qichi.as_mut().on_refresh_failure(err);
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
            rust.library_cache.clear();
            rust.library_loading = false;
            rust.library_request_running = false;
            rust.library_loaded = false;
            rust.library_error = QString::default();
            rust.library_refresh_at = None;
            rust.library_refresh_pending = None;
            rust.library_revision = rust.library_revision.wrapping_add(1);
        }
        self.as_mut().notify();
    }

    /// 文件写操作收尾后补发期间用户点过的资料库手动刷新。
    pub(crate) fn resume_pending_library_refresh(mut self: Pin<&mut Self>) {
        let pending = {
            let mut rust = self.as_mut().rust_mut();
            let pending = rust.library_refresh_pending.take();
            pending.filter(|kind| rust.library_kind.to_string() == kind.key())
        };
        if let Some(kind) = pending {
            self.refresh_library_inner(QString::from(kind.key()), false, true);
        }
    }

    /// 资料库星标先改当前集合与对应缓存；请求失败再用快照恢复。
    pub(crate) fn optimistic_library_starred(
        mut self: Pin<&mut Self>,
        ids: &[String],
        starred: bool,
    ) -> Option<LibraryStarSnapshot> {
        let kind = LibraryKind::from_key(&self.library_kind.to_string())?;
        let wanted: HashSet<String> = ids
            .iter()
            .map(|id| id.trim())
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
            .collect();
        if wanted.is_empty() {
            return None;
        }

        let now = Instant::now();
        let (snapshot, changed) = {
            let mut rust = self.as_mut().rust_mut();
            let snapshot = LibraryStarSnapshot {
                kind,
                entries: rust.library_entries.clone(),
                rows: rust.library_rows.clone(),
                refresh_at: rust.library_refresh_at,
                cache: rust.library_cache.get(kind.key()).cloned(),
            };
            let changed_view =
                update_starred_entries(&mut rust.library_entries, kind, &wanted, starred);
            if changed_view {
                rust.library_rows = library_view::list_from(&rust.library_entries, kind);
                rust.library_revision = rust.library_revision.wrapping_add(1);
                rust.library_refresh_at = Some(now);
            }
            let mut changed_cache = false;
            if let Some(cache) = rust.library_cache.get_mut(kind.key()) {
                if update_starred_entries(&mut cache.entries, kind, &wanted, starred) {
                    cache.rows = library_view::list_from(&cache.entries, kind);
                    cache.refresh_at = Some(now);
                    changed_cache = true;
                }
            }
            (snapshot, changed_view || changed_cache)
        };
        if !changed {
            return None;
        }
        self.as_mut().notify();
        Some(snapshot)
    }

    pub(crate) fn rollback_library_starred(
        mut self: Pin<&mut Self>,
        snapshot: LibraryStarSnapshot,
    ) {
        {
            let mut rust = self.as_mut().rust_mut();
            if rust.library_kind.to_string() == snapshot.kind.key() {
                rust.library_entries = snapshot.entries;
                rust.library_rows = snapshot.rows;
                rust.library_refresh_at = snapshot.refresh_at;
                rust.library_revision = rust.library_revision.wrapping_add(1);
            }
            if let Some(cache) = snapshot.cache {
                rust.library_cache.insert(snapshot.kind.key(), cache);
            }
        }
        self.as_mut().notify();
    }

    /// 文件写操作成功后让资料库缓存后台校准；`except` 是已经就地更新过的集合。
    pub(crate) fn mark_library_caches_stale(mut self: Pin<&mut Self>, except: Option<LibraryKind>) {
        let mut rust = self.as_mut().rust_mut();
        // 操作前发出的资料库请求不能把旧快照写回来。
        if rust.library_request_running {
            rust.library_epoch = rust.library_epoch.wrapping_add(1);
            rust.library_loading = false;
            rust.library_request_running = false;
        }
        for (kind, cache) in &mut rust.library_cache {
            if except.is_none_or(|updated| updated.key() != *kind) {
                cache.refresh_at = None;
            }
        }
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

fn clear_library_view(rust: &mut super::KichiRust, kind: LibraryKind) {
    rust.library_kind = QString::from(kind.key());
    rust.library_rows = QVariantList::default();
    rust.library_entries.clear();
    rust.library_loading = false;
    rust.library_request_running = false;
    rust.library_loaded = false;
    rust.library_error = QString::default();
    rust.library_refresh_at = None;
    rust.library_revision = rust.library_revision.wrapping_add(1);
}

fn restore_library_cache(rust: &mut super::KichiRust, kind: LibraryKind) -> bool {
    let Some(cache) = rust.library_cache.get(kind.key()) else {
        return false;
    };
    rust.library_kind = QString::from(kind.key());
    rust.library_rows = cache.rows.clone();
    rust.library_entries = cache.entries.clone();
    rust.library_loading = false;
    rust.library_request_running = false;
    rust.library_loaded = cache.loaded;
    rust.library_error = QString::from(cache.error.as_str());
    rust.library_refresh_at = cache.refresh_at;
    rust.library_revision = rust.library_revision.wrapping_add(1);
    true
}

fn library_cache_is_fresh(rust: &super::KichiRust, kind: LibraryKind, now: Instant) -> bool {
    rust.library_cache
        .get(kind.key())
        .is_some_and(|cache| cache.is_fresh(now))
}

fn begin_library_refresh(
    rust: &mut super::KichiRust,
    kind: LibraryKind,
    now: Instant,
    visible: bool,
) -> u64 {
    rust.library_epoch = rust.library_epoch.wrapping_add(1);
    rust.library_kind = QString::from(kind.key());
    rust.library_request_running = true;
    rust.library_loading = visible;
    rust.library_refresh_at = Some(now);
    if visible {
        rust.library_error = QString::default();
    }
    rust.library_epoch
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

    #[test]
    fn cached_library_snapshot_expires_after_ttl() {
        let now = Instant::now();
        let mut cache = LibraryCache {
            rows: QVariantList::default(),
            entries: vec![entry("a", true)],
            loaded: true,
            error: String::new(),
            refresh_at: Some(now - LIBRARY_REFRESH_TTL + Duration::from_millis(1)),
        };
        assert!(cache.is_fresh(now));

        cache.refresh_at = Some(now - LIBRARY_REFRESH_TTL);
        assert!(!cache.is_fresh(now));

        cache.refresh_at = Some(now);
        cache.loaded = false;
        assert!(!cache.is_fresh(now));
    }
}
