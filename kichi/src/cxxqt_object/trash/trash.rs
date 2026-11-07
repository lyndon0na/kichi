//! 回收站：全账号列表、还原、彻底删除与清空。
//!
//! 普通文件页仍然只调 `batchTrash`。`batchDelete` 与 `trash:empty`
//! 只能从本模块进入，且 QML 必须先完成危险操作确认。

use std::pin::Pin;
use std::time::{Duration, Instant};

use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;
use kichi_core::TrashEntry;

use crate::session::page_snapshot;

use super::actions::run_in_background;
use super::file_action_lock::sync_file_action_properties;
use super::qobject;
use super::QVariantList;

#[path = "trash_actions.rs"]
mod trash_actions;
#[path = "trash_mutation.rs"]
mod trash_mutation;
#[path = "trash_view.rs"]
mod trash_view;

pub(crate) use trash_actions::{TrashActionKind, TrashActionState};
use trash_view::{sort_trash_entries, trash_list_from};

/// 切回回收站时，列表缓存超过这个时间才在后台校准。
const TRASH_REFRESH_TTL: Duration = Duration::from_secs(30);

impl qobject::Kichi {
    /// 重新读取全账号回收站。
    pub fn refresh_trash(mut self: Pin<&mut Self>) {
        self.as_mut().refresh_trash_with_status(None, true);
    }

    /// 页面重新可见：有缓存且仍新鲜时不再发请求。
    pub fn ensure_trash_fresh(self: Pin<&mut Self>) {
        let now = Instant::now();
        let (loaded, last) = {
            let qichi = self.as_ref();
            let rust = qichi.rust();
            (rust.trash_loaded, rust.trash_refresh_at)
        };
        if loaded && last.is_some_and(|at| now.saturating_duration_since(at) < TRASH_REFRESH_TTL) {
            return;
        }
        self.refresh_trash_with_status(None, false);
    }

    /// 退出登录时清空回收站状态，并让在途请求作废。
    pub(crate) fn reset_trash(mut self: Pin<&mut Self>) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.trash_epoch = rust.trash_epoch.wrapping_add(1);
            rust.trash_rows = QVariantList::default();
            rust.trash_list.clear();
            rust.trash_loading = false;
            rust.trash_request_running = false;
            rust.trash_loaded = false;
            rust.trash_refresh_at = None;
            rust.trash_refresh_pending = false;
            rust.trash_error = QString::default();
            rust.trash_revision = rust.trash_revision.wrapping_add(1);
            rust.trash_actions.clear();
            rust.files_stale = false;
            sync_file_action_properties(&mut rust);
        }
        self.as_mut().notify();
    }

    /// 文件页的「移入回收站」成功后让缓存失效；下次进入回收站会重新拉取。
    pub(crate) fn mark_trash_stale(mut self: Pin<&mut Self>) {
        let mut rust = self.as_mut().rust_mut();
        // 让操作开始前发出的回收站请求失效，避免它把旧快照写回来。
        rust.trash_epoch = rust.trash_epoch.wrapping_add(1);
        rust.trash_loading = false;
        rust.trash_request_running = false;
        rust.trash_loaded = false;
        rust.trash_refresh_at = None;
        rust.trash_refresh_pending = false;
    }

    pub(super) fn refresh_trash_with_status(
        mut self: Pin<&mut Self>,
        final_status: Option<String>,
        manual: bool,
    ) {
        if self.as_ref().rust().has_trash_write() {
            self.as_mut().rust_mut().trash_refresh_pending = true;
            return;
        }
        let mut has_cache = self.trash_loaded || !self.trash_list.is_empty();
        if self.trash_request_running {
            self.as_mut().rust_mut().trash_refresh_pending = true;
            if manual && has_cache && !self.trash_loading {
                self.as_mut().rust_mut().trash_loading = true;
                self.as_mut().notify();
            }
            return;
        }
        // 会话内没有快照时先看本地索引：回收站快照仍新鲜就直接渲染。
        if !has_cache {
            if let Some(index) = self.backend.session.tree_index() {
                let now = chrono::Utc::now().timestamp();
                match index.trash_listing_if_fresh(now, crate::session::tree_index::FRESH_TTL_SECS)
                {
                    Ok(Some(mut entries)) => {
                        sort_trash_entries(&mut entries);
                        let files = trash_list_from(&entries);
                        {
                            let mut rust = self.as_mut().rust_mut();
                            rust.trash_rows = files;
                            rust.trash_list = entries;
                            rust.trash_revision = rust.trash_revision.wrapping_add(1);
                            rust.trash_loading = false;
                            rust.trash_request_running = false;
                            rust.trash_loaded = true;
                            rust.trash_refresh_at = Some(Instant::now());
                            rust.trash_error = QString::default();
                            if let Some(status) = final_status {
                                rust.status_message = QString::from(status.as_str());
                            }
                        }
                        self.as_mut().notify();
                        return;
                    }
                    Ok(None) => {}
                    Err(err) => {
                        self.backend
                            .session
                            .record_tree_index_error(format!("本地索引读取失败：{err}"));
                    }
                }
            }
        }
        // 索引过期或还没建好时，用上次成功列表的快照先占位，再静默校准。
        if !has_cache {
            let session = self.backend.session.clone();
            if let Some(mut entries) =
                page_snapshot::load::<Vec<TrashEntry>>(&session, page_snapshot::TRASH)
            {
                sort_trash_entries(&mut entries);
                let files = trash_list_from(&entries);
                {
                    let mut rust = self.as_mut().rust_mut();
                    rust.trash_rows = files;
                    rust.trash_list = entries;
                    rust.trash_revision = rust.trash_revision.wrapping_add(1);
                    rust.trash_loading = false;
                    rust.trash_request_running = false;
                    rust.trash_loaded = true;
                    // 快照只负责占位：校准仍需走网络。
                    rust.trash_refresh_at = None;
                    rust.trash_error = QString::default();
                }
                self.as_mut().notify();
                has_cache = true;
            }
        }
        let session = self.backend.session.clone();
        let (epoch, visible) = {
            let mut rust = self.as_mut().rust_mut();
            let visible = manual || !has_cache;
            rust.trash_loading = visible;
            rust.trash_request_running = true;
            rust.trash_refresh_pending = false;
            if visible {
                rust.trash_error = QString::default();
            }
            rust.trash_refresh_at = Some(Instant::now());
            rust.trash_epoch = rust.trash_epoch.wrapping_add(1);
            (rust.trash_epoch, visible)
        };
        if visible {
            self.as_mut().notify();
        }

        run_in_background(
            move || {
                let entries = session.list_trash()?;
                // 网络成功后才替换本地快照；索引失败只记录，不影响回收站展示。
                session.write_index(|index| index.replace_trash(&entries));
                page_snapshot::save(&session, page_snapshot::TRASH, &entries);
                Ok(entries)
            },
            move |mut qichi, result| {
                if qichi.as_ref().rust().trash_epoch != epoch {
                    return;
                }
                match result {
                    Ok(mut entries) => {
                        qichi.as_mut().report_index_error();
                        sort_trash_entries(&mut entries);
                        let files = trash_list_from(&entries);
                        let notify;
                        {
                            let mut rust = qichi.as_mut().rust_mut();
                            let changed = rust.trash_list != entries;
                            if changed {
                                rust.trash_rows = files;
                                rust.trash_list = entries;
                                rust.trash_revision = rust.trash_revision.wrapping_add(1);
                            }
                            rust.trash_loading = false;
                            rust.trash_request_running = false;
                            rust.trash_loaded = true;
                            rust.trash_refresh_at = Some(Instant::now());
                            rust.trash_error = QString::default();
                            if let Some(status) = final_status {
                                rust.status_message = QString::from(status.as_str());
                            }
                            notify = changed || visible;
                        }
                        if notify {
                            qichi.as_mut().notify();
                        }
                        qichi.as_mut().resume_pending_trash_refresh();
                    }
                    Err(err) => {
                        let text = err.to_string();
                        {
                            let mut rust = qichi.as_mut().rust_mut();
                            rust.trash_loading = false;
                            rust.trash_request_running = false;
                            rust.trash_refresh_at = Some(Instant::now());
                            if !has_cache {
                                rust.trash_error = QString::from(text.as_str());
                            }
                        }
                        qichi.as_mut().on_refresh_failure(err);
                        qichi.as_mut().resume_pending_trash_refresh();
                    }
                }
            },
        );
    }

    /// 回收站请求 / 写操作期间又收到的刷新，只在最后一个动作收尾后补一次。
    pub(super) fn resume_pending_trash_refresh(mut self: Pin<&mut Self>) {
        let pending = {
            let mut rust = self.as_mut().rust_mut();
            if rust.trash_refresh_pending && !rust.has_trash_write() && !rust.trash_request_running
            {
                rust.trash_refresh_pending = false;
                true
            } else {
                false
            }
        };
        if pending {
            self.as_mut().refresh_trash_with_status(None, false);
        }
    }
}

#[cfg(test)]
#[path = "trash_tests.rs"]
mod tests;
