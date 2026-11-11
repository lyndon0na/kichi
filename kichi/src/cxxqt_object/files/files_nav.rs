//! 目录导航：进出文件夹、面包屑、任务产物定位。

use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;
use kichi_core::{EntryKind, TaskPhase};

use super::actions::run_in_background;
use super::file_action_lock::reset_file_action_state;
use super::file_optimistic::is_unconfirmed_create;
use super::files::Crumb;
use super::qobject;
use super::QVariantList;

impl qobject::Kichi {
    pub fn open_dir(mut self: Pin<&mut Self>, id: QString) {
        if self.listing {
            return;
        }
        let id = id.to_string();
        let found = self
            .entries
            .iter()
            .find(|entry| entry.kind == EntryKind::Folder && entry.id == id)
            .map(|entry| entry.name.clone());
        let Some(name) = found else {
            self.as_mut()
                .set_message("该文件夹已不在当前列表中，请刷新后重试");
            return;
        };
        if is_unconfirmed_create(self.as_ref().rust(), &id) {
            self.as_mut()
                .set_message("文件夹正在同步真实 id，请稍后再打开");
            return;
        }
        self.as_mut().clear_status_message();
        {
            let mut rust = self.as_mut().rust_mut();
            rust.crumb_backup = Some(rust.crumbs.clone());
            rust.crumbs.push(Crumb { id, name });
        }
        self.as_mut().sync_breadcrumb();
        self.as_mut().enter_current_dir();
    }

    /// 从离线任务打开已完成的产物文件夹。
    ///
    /// 任务响应只给产物 id，不给完整祖先路径。后台先逐级反查目录链，再读取
    /// 目标文件夹内容；两者都成功后才把新路径和列表一次性写回并通知界面切页。
    pub fn open_task_folder(mut self: Pin<&mut Self>, id: QString) -> bool {
        if self.listing {
            self.as_mut().set_message("文件列表正在加载，请稍后再打开");
            return false;
        }
        let id = id.to_string();
        let target = self
            .task_list
            .iter()
            .find(|task| {
                task.file_id.as_deref() == Some(id.as_str())
                    && task.kind == Some(EntryKind::Folder)
                    && task.phase == TaskPhase::Completed
            })
            .map(|task| (id.clone(), task.name.clone()));
        let Some((file_id, task_name)) = target else {
            self.as_mut()
                .set_message("该离线任务已完成文件夹不在当前列表中，请刷新后重试");
            return false;
        };
        self.as_mut().exit_search();
        let session = self.backend.session.clone();
        let previous_crumbs = self.crumbs.clone();
        let epoch = {
            let mut rust = self.as_mut().rust_mut();
            rust.listing = true;
            rust.task_folder_loading_id = QString::from(file_id.as_str());
            rust.refresh_pending = false;
            rust.refresh_pending_manual = false;
            rust.list_epoch = rust.list_epoch.wrapping_add(1);
            rust.list_epoch
        };
        self.as_mut()
            .set_message(&format!("正在打开「{task_name}」……"));
        run_in_background(
            move || {
                let trail = session.folder_trail(&file_id)?;
                let folder_id = trail
                    .last()
                    .map(|(id, _)| id.clone())
                    .ok_or_else(|| crate::session::Failure::Other("目录路径为空".to_owned()))?;
                let entries = session.list_dir(Some(&folder_id))?;
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
                            rust.crumbs = vec![Crumb::root()];
                            rust.crumbs
                                .extend(trail.into_iter().map(|(id, name)| Crumb { id, name }));
                        }
                        qichi.as_mut().sync_breadcrumb();
                        qichi.as_mut().clear_status_message();
                        // 列表已经在后台读到，show_entries 会同时清掉 loading 并写入 files。
                        qichi.as_mut().show_entries(entries);
                        qichi.as_mut().task_folder_ready();
                    }
                    Err(err) => {
                        let mut rust = qichi.as_mut().rust_mut();
                        rust.listing = false;
                        rust.task_folder_loading_id = QString::default();
                        rust.crumbs = previous_crumbs;
                        qichi.as_mut().on_failure(err);
                    }
                }
            },
        );
        true
    }

    /// 从离线任务跳转到产物所在父目录，并通知文件页选中目标。
    ///
    /// 与 [`Self::open_task_folder`] 的区别：
    /// - 文件夹本人不进入，而是读取它的父目录；
    /// - 文件和文件夹都适用；
    /// - 父目录加载完成后发 `taskLocationReady(targetId)`，由 QML 选中目标。
    pub fn reveal_task_entry(mut self: Pin<&mut Self>, id: QString) -> bool {
        if self.listing {
            self.as_mut().set_message("文件列表正在加载，请稍后再定位");
            return false;
        }
        let target_id = id.to_string();
        let target = self
            .task_list
            .iter()
            .find(|task| {
                task.file_id.as_deref() == Some(target_id.as_str())
                    && task.phase == TaskPhase::Completed
                    && matches!(task.kind, Some(EntryKind::File | EntryKind::Folder))
            })
            .and_then(|task| task.kind.map(|kind| (kind, task.name.clone())));
        let Some((expected_kind, task_name)) = target else {
            self.as_mut()
                .set_message("该离线任务已完成产物不在当前列表中，请刷新后重试");
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
            rust.refresh_pending_manual = false;
            rust.list_epoch = rust.list_epoch.wrapping_add(1);
            rust.list_epoch
        };
        self.as_mut()
            .set_message(&format!("正在定位「{task_name}」……"));

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
                            rust.crumbs = vec![Crumb::root()];
                            rust.crumbs
                                .extend(trail.into_iter().map(|(id, name)| Crumb { id, name }));
                        }
                        qichi.as_mut().sync_breadcrumb();
                        qichi.as_mut().clear_status_message();
                        qichi.as_mut().show_entries(entries);
                        qichi
                            .as_mut()
                            .task_location_ready(QString::from(target_for_result.as_str()));
                    }
                    Err(err) => {
                        let mut rust = qichi.as_mut().rust_mut();
                        rust.listing = false;
                        rust.task_folder_loading_id = QString::default();
                        rust.crumbs = previous_crumbs;
                        qichi.as_mut().on_failure(err);
                    }
                }
            },
        );
        true
    }

    /// 回到上一级。
    pub fn open_parent(mut self: Pin<&mut Self>) {
        if self.listing && !self.files_request_running {
            return;
        }
        let moved = {
            let mut rust = self.as_mut().rust_mut();
            if rust.crumbs.len() > 1 {
                rust.crumb_backup = Some(rust.crumbs.clone());
                rust.crumbs.pop();
                true
            } else {
                false
            }
        };
        if !moved {
            return;
        }
        self.as_mut().clear_status_message();
        self.as_mut().sync_breadcrumb();
        self.as_mut().enter_current_dir();
    }

    /// 跳到面包屑上的某一级（点击当前目录本身无效果）。
    pub fn open_crumb(mut self: Pin<&mut Self>, index: i32) {
        if index < 0 || self.listing && !self.files_request_running {
            return;
        }
        let target = index as usize;
        let len = self.crumbs.len();
        if len < 2 || target + 1 >= len {
            return;
        }
        self.as_mut().clear_status_message();
        {
            let mut rust = self.as_mut().rust_mut();
            rust.crumb_backup = Some(rust.crumbs.clone());
            rust.crumbs.truncate(target + 1);
        }
        self.as_mut().sync_breadcrumb();
        self.as_mut().enter_current_dir();
    }

    /// 退出登录时清空浏览状态并让在途请求作废。
    pub(crate) fn reset_files(mut self: Pin<&mut Self>) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.list_epoch = rust.list_epoch.wrapping_add(1);
            rust.listing = false;
            rust.task_folder_loading_id = QString::default();
            rust.refresh_pending = false;
            rust.refresh_pending_manual = false;
            rust.entries.clear();
            rust.files = QVariantList::default();
            rust.filtered_files = QVariantList::default();
            rust.files_revision = rust.files_revision.wrapping_add(1);
            rust.files_cache.clear();
            rust.files_request_running = false;
            rust.files_manual_refreshing = false;
            rust.unresolved_create.clear();
            rust.pending_entry_ids.clear();
            rust.crumbs = vec![Crumb::root()];
            rust.crumb_backup = None;
            // 选择器也一并清空：旧账号的目录不能留在界面上
            rust.move_targets_epoch = rust.move_targets_epoch.wrapping_add(1);
            rust.move_targets = QVariantList::default();
            rust.move_targets_loading = false;
            rust.move_targets_dir = String::new();
            rust.move_targets_error = QString::default();
            rust.move_targets_revision = rust.move_targets_revision.wrapping_add(1);
            reset_file_action_state(&mut rust);
        }
        self.as_mut().sync_breadcrumb();
    }
}
