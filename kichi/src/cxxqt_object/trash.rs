//! 回收站：全账号列表、还原、彻底删除与清空。
//!
//! 普通文件页仍然只调 `batchTrash`。`batchDelete` 与 `trash:empty`
//! 只能从本模块进入，且 QML 必须先完成危险操作确认。

use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;

use super::actions::run_in_background;
use super::qobject;
use super::QVariantList;

#[path = "trash_actions.rs"]
mod trash_actions;
#[path = "trash_view.rs"]
mod trash_view;

pub(crate) use trash_actions::TrashActionState;
use trash_view::{sort_trash_entries, trash_list_from};

impl qobject::Kichi {
    /// 重新读取全账号回收站。
    pub fn refresh_trash(mut self: Pin<&mut Self>) {
        self.as_mut().refresh_trash_with_status(None);
    }

    /// 退出登录时清空回收站状态，并让在途请求作废。
    pub(crate) fn reset_trash(mut self: Pin<&mut Self>) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.trash_epoch = rust.trash_epoch.wrapping_add(1);
            rust.trash_rows = QVariantList::default();
            rust.trash_list.clear();
            rust.trash_loading = false;
            rust.trash_loaded = false;
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
        self.as_mut().rust_mut().trash_loaded = false;
    }

    pub(super) fn refresh_trash_with_status(
        mut self: Pin<&mut Self>,
        final_status: Option<String>,
    ) {
        if self.trash_loading || self.file_action_running {
            return;
        }
        let session = self.backend.session.clone();
        let epoch = {
            let mut rust = self.as_mut().rust_mut();
            rust.trash_loading = true;
            rust.trash_error = QString::default();
            rust.trash_epoch = rust.trash_epoch.wrapping_add(1);
            rust.trash_epoch
        };
        self.as_mut().notify();

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
                        {
                            let mut rust = qichi.as_mut().rust_mut();
                            rust.trash_rows = files;
                            rust.trash_list = entries;
                            rust.trash_loading = false;
                            rust.trash_loaded = true;
                            rust.trash_error = QString::default();
                            rust.trash_revision = rust.trash_revision.wrapping_add(1);
                            if let Some(status) = final_status {
                                rust.status_message = QString::from(status.as_str());
                            }
                        }
                        qichi.as_mut().notify();
                    }
                    Err(err) => {
                        let text = err.to_string();
                        {
                            let mut rust = qichi.as_mut().rust_mut();
                            rust.trash_loading = false;
                            rust.trash_error = QString::from(text.as_str());
                        }
                        qichi.as_mut().on_failure(err);
                    }
                }
            },
        );
    }
}

#[cfg(test)]
#[path = "trash_tests.rs"]
mod tests;
