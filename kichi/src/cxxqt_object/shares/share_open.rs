//! 打开别人的分享：解析链接、浏览目录、转存到自己的网盘。
//!
//! 网络请求都在后台线程；界面状态由 `share_open_*` 一组属性驱动。
//! 每次请求带代际（epoch），关闭弹窗 / 退出登录后旧结果直接丢弃。

use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QMap, QMapPair_QString_QVariant, QString, QStringList, QVariant};
use kichi_core::{
    parse_share_input, EntryKind, FileEntry, RestoreShareRequest, ShareListing, ShareOpenInfo,
};

use super::actions::run_in_background;
use super::qobject;
use super::QVariantList;
use crate::session::Failure;

type QVariantMap = QMap<QMapPair_QString_QVariant>;

/// 浏览栈的一层（`id` 为空 = 分享顶层）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ShareCrumb {
    pub id: String,
    pub name: String,
}

impl qobject::Kichi {
    /// 解析并打开一段分享文本 / 链接。`pass_code` 为空时用文本里带的密码。
    pub fn open_share_link(mut self: Pin<&mut Self>, text: QString, pass_code: QString) {
        let raw = text.to_string();
        let typed_code = pass_code.to_string();
        let typed_code = typed_code.trim();

        let input = match parse_share_input(&raw) {
            Ok(input) => input,
            Err(err) => {
                {
                    let mut rust = self.as_mut().rust_mut();
                    rust.share_open_epoch = rust.share_open_epoch.wrapping_add(1);
                    clear_share_open(&mut rust);
                    rust.share_open_error = QString::from(err.to_string().as_str());
                }
                self.as_mut().notify();
                return;
            }
        };
        let share_id = input.share_id;
        let pass_code = if typed_code.is_empty() {
            input.pass_code
        } else {
            Some(typed_code.to_owned())
        };
        let session = self.backend.session.clone();

        let epoch = {
            let mut rust = self.as_mut().rust_mut();
            rust.share_open_epoch = rust.share_open_epoch.wrapping_add(1);
            clear_share_open(&mut rust);
            rust.share_open_busy = true;
            rust.share_open_epoch
        };
        self.as_mut().notify();

        run_in_background(
            move || session.open_share(&share_id, pass_code.as_deref()),
            move |mut qichi, result| {
                if qichi.as_ref().rust().share_open_epoch != epoch {
                    return;
                }
                match result {
                    Ok(info) => qichi.as_mut().show_share_opened(info),
                    Err(err) => qichi.as_mut().show_share_open_failure(err),
                }
            },
        );
    }

    /// 进入分享内的文件夹。
    pub fn open_share_folder(mut self: Pin<&mut Self>, folder_id: QString, folder_name: QString) {
        if self.share_open_busy || !self.share_open_ready {
            return;
        }
        let folder_id = folder_id.to_string();
        let folder_id = folder_id.trim();
        if folder_id.is_empty() {
            return;
        }
        let folder_name = folder_name.to_string();
        let name = if folder_name.trim().is_empty() {
            "未命名文件夹".to_owned()
        } else {
            folder_name.trim().to_owned()
        };
        let mut stack = self.share_open_stack.clone();
        stack.push(ShareCrumb {
            id: folder_id.to_owned(),
            name,
        });
        self.as_mut().load_share_dir(stack);
    }

    /// 「上一级」：回到浏览栈的上一层。
    pub fn share_open_go_up(mut self: Pin<&mut Self>) {
        if self.share_open_busy || !self.share_open_ready {
            return;
        }
        let mut stack = self.share_open_stack.clone();
        if stack.len() <= 1 {
            return;
        }
        stack.pop();
        self.as_mut().load_share_dir(stack);
    }

    /// 关闭弹窗：丢弃在途结果并清空浏览状态。
    pub fn close_share_open(mut self: Pin<&mut Self>) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.share_open_epoch = rust.share_open_epoch.wrapping_add(1);
            rust.share_save_epoch = rust.share_save_epoch.wrapping_add(1);
            clear_share_open(&mut rust);
            rust.share_save_running = false;
        }
        self.as_mut().notify();
    }

    /// 把选中条目转存到自己的某个目录（空 `parent_id` = 根目录）。
    ///
    /// 返回 false = 请求没有被接受（未选中 / 已有转存在途），界面留在原地。
    pub fn restore_share_files(
        mut self: Pin<&mut Self>,
        file_ids: QStringList,
        parent_id: QString,
    ) -> bool {
        if self.share_save_running {
            return false;
        }
        let ids = clean_ids(file_ids);
        if ids.is_empty() {
            self.as_mut()
                .share_restore_done(false, QString::from("没有选中要转存的条目"));
            return false;
        }
        let share_id = self.share_open_share_id.trim().to_owned();
        let pass_code_token = self.share_open_pass_code_token.trim().to_owned();
        if share_id.is_empty() || pass_code_token.is_empty() {
            self.as_mut()
                .share_restore_done(false, QString::from("分享还没有打开，无法转存"));
            return false;
        }
        // 分享内从顶层到当前目录的文件夹链；顶层条目为空。
        let ancestor_ids: Vec<String> = self
            .share_open_stack
            .iter()
            .skip(1)
            .map(|crumb| crumb.id.clone())
            .filter(|id| !id.is_empty())
            .collect();
        let request = RestoreShareRequest {
            share_id,
            pass_code_token,
            file_ids: ids,
            parent_id: parent_id.to_string().trim().to_owned(),
            ancestor_ids,
        };
        let target_parent = request.parent_id.clone();
        let session = self.backend.session.clone();
        let epoch = {
            let mut rust = self.as_mut().rust_mut();
            rust.share_save_epoch = rust.share_save_epoch.wrapping_add(1);
            rust.share_save_running = true;
            rust.share_save_epoch
        };
        self.as_mut().notify();

        run_in_background(
            move || session.restore_share(&request),
            move |mut qichi, result| {
                if qichi.as_ref().rust().share_save_epoch != epoch {
                    return;
                }
                qichi.as_mut().rust_mut().share_save_running = false;
                match result {
                    Ok(outcome) => {
                        // 目标目录里已经（或即将）出现新条目：下次进入文件页先校准。
                        qichi.as_mut().mark_file_dir_stale(&target_parent);
                        let message = if outcome.completed {
                            "已转存到所选文件夹"
                        } else {
                            "转存已开始，完成后会出现在目标文件夹"
                        };
                        qichi.as_mut().notify();
                        qichi
                            .as_mut()
                            .share_restore_done(true, QString::from(message));
                    }
                    Err(failure) => {
                        let text = failure.to_string();
                        // 业务失败只在弹窗里说明；登录 / 验证 / 限流仍走统一出口。
                        if !is_local_failure(&failure) {
                            qichi.as_mut().on_failure(failure);
                        }
                        qichi.as_mut().notify();
                        qichi
                            .as_mut()
                            .share_restore_done(false, QString::from(text.as_str()));
                    }
                }
            },
        );
        true
    }

    /// 退出登录时清空打开分享的全部状态（不发通知，调用方随后统一 notify）。
    pub(crate) fn reset_share_open(mut self: Pin<&mut Self>) {
        let mut rust = self.as_mut().rust_mut();
        rust.share_open_epoch = rust.share_open_epoch.wrapping_add(1);
        rust.share_save_epoch = rust.share_save_epoch.wrapping_add(1);
        clear_share_open(&mut rust);
        rust.share_save_running = false;
    }

    fn load_share_dir(mut self: Pin<&mut Self>, stack: Vec<ShareCrumb>) {
        let share_id = self.share_open_share_id.trim().to_owned();
        let pass_code_token = self.share_open_pass_code_token.trim().to_owned();
        if share_id.is_empty() || pass_code_token.is_empty() {
            return;
        }
        let parent_id = stack
            .last()
            .map(|crumb| crumb.id.clone())
            .filter(|id| !id.is_empty());
        let session = self.backend.session.clone();
        let epoch = {
            let mut rust = self.as_mut().rust_mut();
            rust.share_open_epoch = rust.share_open_epoch.wrapping_add(1);
            rust.share_open_busy = true;
            rust.share_open_error = QString::default();
            rust.share_open_epoch
        };
        self.as_mut().notify();

        run_in_background(
            move || session.share_folder(&share_id, &pass_code_token, parent_id.as_deref()),
            move |mut qichi, result| {
                if qichi.as_ref().rust().share_open_epoch != epoch {
                    return;
                }
                match result {
                    Ok(listing) => qichi.as_mut().show_share_listing(stack, listing),
                    Err(err) => qichi.as_mut().show_share_browse_failure(err),
                }
            },
        );
    }

    fn show_share_opened(mut self: Pin<&mut Self>, info: ShareOpenInfo) {
        let rows = entry_rows(&info.entries);
        let root = ShareCrumb {
            id: String::new(),
            name: "全部文件".to_owned(),
        };
        let path = path_list(std::slice::from_ref(&root));
        let meta = meta_label(&info);
        let owner = if info.owner.trim().is_empty() {
            QString::default()
        } else {
            QString::from(format!("来自 {}", info.owner.trim()).as_str())
        };
        {
            let mut rust = self.as_mut().rust_mut();
            rust.share_open_busy = false;
            rust.share_open_ready = true;
            rust.share_open_needs_pass_code = false;
            rust.share_open_error = QString::default();
            rust.share_open_title = QString::from(info.title.trim());
            rust.share_open_owner = owner;
            rust.share_open_meta = QString::from(meta.as_str());
            rust.share_open_rows = rows;
            rust.share_open_path = path;
            rust.share_open_truncated = info.truncated;
            rust.share_open_share_id = info.share_id;
            rust.share_open_pass_code_token = info.pass_code_token;
            rust.share_open_stack = vec![root];
        }
        self.as_mut().notify();
    }

    fn show_share_listing(mut self: Pin<&mut Self>, stack: Vec<ShareCrumb>, listing: ShareListing) {
        let rows = entry_rows(&listing.entries);
        {
            let mut rust = self.as_mut().rust_mut();
            rust.share_open_busy = false;
            rust.share_open_needs_pass_code = false;
            rust.share_open_error = QString::default();
            rust.share_open_rows = rows;
            rust.share_open_path = path_list(&stack);
            rust.share_open_truncated = listing.truncated;
            rust.share_open_stack = stack;
        }
        self.as_mut().notify();
    }

    fn show_share_open_failure(mut self: Pin<&mut Self>, failure: Failure) {
        let needs_code = failure.needs_pass_code();
        let text = failure.to_string();
        {
            let mut rust = self.as_mut().rust_mut();
            rust.share_open_busy = false;
            rust.share_open_ready = false;
            rust.share_open_needs_pass_code = needs_code;
            rust.share_open_error = QString::from(text.as_str());
        }
        self.as_mut().notify();
        if !is_local_failure(&failure) {
            self.as_mut().on_failure(failure);
        }
    }

    fn show_share_browse_failure(mut self: Pin<&mut Self>, failure: Failure) {
        let text = failure.to_string();
        let needs_code = failure.needs_pass_code();
        {
            let mut rust = self.as_mut().rust_mut();
            rust.share_open_busy = false;
            rust.share_open_needs_pass_code = needs_code;
            rust.share_open_error = QString::from(text.as_str());
        }
        self.as_mut().notify();
        if !is_local_failure(&failure) {
            self.as_mut().on_failure(failure);
        }
    }
}

/// 只在弹窗里显示的失败：分享本身的问题 + 普通业务失败。
fn is_local_failure(failure: &Failure) -> bool {
    matches!(
        failure,
        Failure::ShareUnavailable { .. } | Failure::Other(_)
    )
}

fn clear_share_open(rust: &mut super::KichiRust) {
    rust.share_open_busy = false;
    rust.share_open_ready = false;
    rust.share_open_needs_pass_code = false;
    rust.share_open_error = QString::default();
    rust.share_open_title = QString::default();
    rust.share_open_owner = QString::default();
    rust.share_open_meta = QString::default();
    rust.share_open_rows = QVariantList::default();
    rust.share_open_path = QVariantList::default();
    rust.share_open_truncated = false;
    rust.share_open_share_id = String::new();
    rust.share_open_pass_code_token = String::new();
    rust.share_open_stack.clear();
}

fn clean_ids(ids: QStringList) -> Vec<String> {
    ids.iter()
        .map(|id| id.to_string().trim().to_owned())
        .filter(|id| !id.is_empty())
        .collect()
}

fn meta_label(info: &ShareOpenInfo) -> String {
    let mut parts = vec![format!("{} 个条目", info.file_count)];
    if info.never_expires {
        parts.push("永久有效".to_owned());
    } else if let Some(at) = info.expiration_at {
        parts.push(format!(
            "{} 到期",
            at.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M")
        ));
    }
    parts.join(" · ")
}

fn entry_rows(entries: &[FileEntry]) -> QVariantList {
    let mut list = QVariantList::default();
    for entry in entries {
        list.append(QVariant::from(&entry_map(entry)));
    }
    list
}

fn entry_map(entry: &FileEntry) -> QVariantMap {
    let folder = entry.kind == EntryKind::Folder;
    let text = |value: &str| QVariant::from(&QString::from(value));
    let size = if folder {
        String::new()
    } else {
        crate::format::size(entry.size)
    };
    let mut map = QVariantMap::default();
    map.insert_clone(&QString::from("id"), &text(&entry.id));
    map.insert_clone(&QString::from("name"), &text(&entry.name));
    map.insert_clone(
        &QString::from("kind"),
        &text(if folder { "folder" } else { "file" }),
    );
    map.insert_clone(&QString::from("isFolder"), &QVariant::from(&folder));
    map.insert_clone(&QString::from("sizeText"), &text(&size));
    map
}

fn path_list(stack: &[ShareCrumb]) -> QVariantList {
    let text = |value: &str| QVariant::from(&QString::from(value));
    let mut list = QVariantList::default();
    for crumb in stack {
        let mut map = QVariantMap::default();
        map.insert_clone(&QString::from("id"), &text(&crumb.id));
        map.insert_clone(&QString::from("name"), &text(&crumb.name));
        list.append(QVariant::from(&map));
    }
    list
}
