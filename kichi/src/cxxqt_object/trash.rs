//! 回收站：全账号列表、还原、彻底删除与清空。
//!
//! 普通文件页仍然只调 `batchTrash`。`batchDelete` 与 `trash:empty`
//! 只能从本模块进入，且 QML 必须先完成危险操作确认。

use std::pin::Pin;
use std::time::{Duration, Instant};

use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;

use super::actions::run_in_background;
use super::qobject;
use super::QVariantList;

#[path = "trash_actions.rs"]
mod trash_actions;
#[path = "trash_mutation.rs"]
mod trash_mutation;
#[path = "trash_view.rs"]
mod trash_view;

pub(crate) use trash_actions::TrashActionState;
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
            rust.trash_error = QString::default();
            rust.trash_revision = rust.trash_revision.wrapping_add(1);
            rust.trash_action = None;
            rust.files_stale = false;
            if rust.file_action_running {
                rust.file_action_running = false;
            }
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
    }

    pub(super) fn refresh_trash_with_status(
        mut self: Pin<&mut Self>,
        final_status: Option<String>,
        manual: bool,
    ) {
        if self.file_action_running {
            return;
        }
        let has_cache = self.trash_loaded || !self.trash_list.is_empty();
        if self.trash_request_running {
            if manual && has_cache && !self.trash_loading {
                self.as_mut().rust_mut().trash_loading = true;
                self.as_mut().notify();
            }
            return;
        }
        let session = self.backend.session.clone();
        let (epoch, visible) = {
            let mut rust = self.as_mut().rust_mut();
            let visible = manual || !has_cache;
            rust.trash_loading = visible;
            rust.trash_request_running = true;
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
            move || session.list_trash(),
            move |mut qichi, result| {
                if qichi.as_ref().rust().trash_epoch != epoch {
                    return;
                }
                match result {
                    Ok(mut entries) => {
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
                    }
                }
            },
        );
    }
}

#[cfg(test)]
#[path = "trash_tests.rs"]
mod tests;
