//! 资料库三页共用的状态、刷新与打开位置。

use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;
use kichi_core::{EntryKind, FileEntry, LibraryKind};

use super::actions::run_in_background;
use super::files::{Crumb, ROOT_NAME};
use super::qobject;
use super::QVariantList;

#[path = "library_view.rs"]
mod library_view;

enum OpenResult {
    Folder {
        trail: Vec<(String, String)>,
        entries: Vec<FileEntry>,
    },
    Location {
        trail: Vec<(String, String)>,
        entries: Vec<FileEntry>,
        target_id: String,
    },
}

impl qobject::Kichi {
    /// 刷新当前资料库页面。与文件目录共用 `filesLoading` 时容易混淆，因此使用
    /// 独立 loading / error 状态；请求按 epoch 作废，快速切页不会贴错集合。
    pub fn refresh_library(mut self: Pin<&mut Self>, kind: QString) {
        let Some(kind) = LibraryKind::from_key(&kind.to_string()) else {
            self.as_mut().set_message("未知的资料库页面");
            return;
        };
        let session = self.backend.session.clone();
        let epoch = {
            let mut rust = self.as_mut().rust_mut();
            rust.library_epoch = rust.library_epoch.wrapping_add(1);
            rust.library_kind = QString::from(kind.key());
            rust.library_rows = QVariantList::default();
            rust.library_entries.clear();
            rust.library_loading = true;
            rust.library_loaded = false;
            rust.library_error = QString::default();
            rust.library_revision = rust.library_revision.wrapping_add(1);
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
            rust.library_revision = rust.library_revision.wrapping_add(1);
        }
        self.as_mut().notify();
    }

    /// 从资料库打开文件夹本身；文件则跳到其父目录并通知界面选中目标。
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

        self.as_mut().exit_search();
        let session = self.backend.session.clone();
        let previous_crumbs = self.crumbs.clone();
        let target_kind = target.kind;
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
                if target_kind == EntryKind::Folder {
                    let trail = session.folder_trail(&work_id)?;
                    let folder_id = trail
                        .last()
                        .map(|(id, _)| id.clone())
                        .ok_or_else(|| crate::session::Failure::Other("目录路径为空".to_owned()))?;
                    let entries = session.list_dir(Some(&folder_id))?;
                    return Ok(OpenResult::Folder { trail, entries });
                }

                let (_, parent_id) = session.parent_of(&work_id)?;
                let (trail, list_parent) = if parent_id.trim().is_empty() {
                    (Vec::new(), None)
                } else {
                    (session.folder_trail(&parent_id)?, Some(parent_id))
                };
                let entries = session.list_dir(list_parent.as_deref())?;
                if !entries.iter().any(|entry| entry.id == work_id) {
                    return Err(crate::session::Failure::Other(
                        "目标条目不在所在目录中，可能已被移动或删除".to_owned(),
                    ));
                }
                Ok(OpenResult::Location {
                    trail,
                    entries,
                    target_id: work_id,
                })
            },
            move |mut qichi, result| {
                if qichi.as_ref().rust().list_epoch != epoch {
                    return;
                }
                match result {
                    Ok(result) => {
                        let target = match &result {
                            OpenResult::Folder { .. } => None,
                            OpenResult::Location { target_id, .. } => Some(target_id.clone()),
                        };
                        let (trail, entries) = match result {
                            OpenResult::Folder { trail, entries } => (trail, entries),
                            OpenResult::Location { trail, entries, .. } => (trail, entries),
                        };
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
                        match target {
                            Some(target) => qichi
                                .as_mut()
                                .library_location_ready(QString::from(target.as_str())),
                            None => qichi.as_mut().library_folder_ready(),
                        }
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
}
