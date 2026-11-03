//! 文件 / 资料库 / 回收站写操作的资源锁。
//!
//! 锁粒度是条目 id：同一目标不能并发写，不同目标可以同时在途。页面刷新只等
//! 自己领域的动作，不被其它页面的慢请求拖住。动作完成回调带目标 id，界面据此
//! 只清理自己那批选择 / 弹窗。

use std::collections::HashSet;
use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QString, QStringList};

use super::qobject::Kichi;
use super::KichiRust;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileActionDomain {
    Files,
    Library,
    Trash,
}

pub(crate) struct FileActionRecord {
    pub(crate) ids: Vec<String>,
    pub(crate) domain: FileActionDomain,
    pub(crate) message: String,
    pub(crate) adds_to_trash: bool,
}

impl Kichi {
    /// 占用本次动作涉及的条目 id；有任一 id 已在别的动作里返回 `None`。
    ///
    /// 锁定和登记必须在同一次 `rust_mut()` 里完成，避免两个入口同时看到“空闲”
    /// 后各写一份乐观状态。
    pub(crate) fn begin_file_action(
        mut self: Pin<&mut Self>,
        ids: &[String],
        domain: FileActionDomain,
    ) -> Option<u64> {
        let ids = unique_ids(ids);
        let action_id = {
            let mut rust = self.as_mut().rust_mut();
            if rust
                .file_action_locks
                .iter()
                .any(|locked| ids.contains(locked))
            {
                return None;
            }
            rust.file_action_seq = rust.file_action_seq.wrapping_add(1);
            let action_id = rust.file_action_seq;
            for id in &ids {
                rust.file_action_locks.insert(id.clone());
            }
            rust.file_action_records.insert(
                action_id,
                FileActionRecord {
                    ids: ids.clone(),
                    domain,
                    message: String::new(),
                    adds_to_trash: false,
                },
            );
            sync_file_action_properties(&mut rust);
            action_id
        };
        self.as_mut().notify();
        Some(action_id)
    }

    /// 新建文件夹在拿到临时 id 之前先登记动作，随后补上要锁的临时行。
    pub(crate) fn add_file_action_target(
        mut self: Pin<&mut Self>,
        action_id: u64,
        id: &str,
    ) -> bool {
        let id = id.trim();
        if id.is_empty() {
            return false;
        }
        let added = {
            let mut rust = self.as_mut().rust_mut();
            if !rust.file_action_records.contains_key(&action_id)
                || rust.file_action_locks.contains(id)
            {
                false
            } else {
                rust.file_action_locks.insert(id.to_owned());
                if let Some(record) = rust.file_action_records.get_mut(&action_id) {
                    record.ids.push(id.to_owned());
                    sync_file_action_properties(&mut rust);
                    true
                } else {
                    false
                }
            }
        };
        if added {
            self.as_mut().notify();
        }
        added
    }

    /// 结束一次文件操作。
    pub(crate) fn end_file_action(
        mut self: Pin<&mut Self>,
        action_id: u64,
        action: &str,
        message: &str,
    ) {
        let owns_message = self.as_ref().owns_file_action_message(action_id);
        let ids = self.as_mut().release_file_action(action_id);
        if action == "trash" {
            if owns_message || self.as_ref().rust().status_message.is_empty() {
                self.as_mut().set_message(message);
            }
        } else if owns_message {
            self.as_mut().clear_status_message();
        }
        let ids = string_list(&ids);
        self.as_mut()
            .file_action_done(QString::from(action), true, QString::from(message), ids);
        self.as_mut().resume_pending_library_refresh();
        self.as_mut().resume_pending_files_request();
        self.as_mut().resume_pending_trash_refresh();
    }

    /// 文件操作失败：释放目标锁 + 走统一的错误出口（分类提示不吞）。
    pub(crate) fn fail_file_action(
        mut self: Pin<&mut Self>,
        action_id: u64,
        action: &str,
        failure: crate::session::Failure,
    ) {
        let ids = self.as_mut().release_file_action(action_id);
        let text = failure.to_string();
        self.as_mut().on_failure(failure);
        let ids = string_list(&ids);
        self.as_mut().file_action_done(
            QString::from(action),
            false,
            QString::from(text.as_str()),
            ids,
        );
        self.as_mut().resume_pending_library_refresh();
        self.as_mut().resume_pending_files_request();
        self.as_mut().resume_pending_trash_refresh();
    }

    pub(crate) fn release_file_action(mut self: Pin<&mut Self>, action_id: u64) -> Vec<String> {
        let ids = {
            let mut rust = self.as_mut().rust_mut();
            let Some(record) = rust.file_action_records.remove(&action_id) else {
                return Vec::new();
            };
            for id in &record.ids {
                rust.file_action_locks.remove(id);
            }
            sync_file_action_properties(&mut rust);
            record.ids
        };
        self.as_mut().notify();
        ids
    }

    /// 标记这次文件页写操作会把条目移入回收站；清空必须等它结束，避免刚移入的条目
    /// 被 `trash:empty` 一并删掉。
    pub(crate) fn mark_file_action_adds_to_trash(mut self: Pin<&mut Self>, action_id: u64) {
        let changed = {
            let mut rust = self.as_mut().rust_mut();
            let changed = rust
                .file_action_records
                .get_mut(&action_id)
                .is_some_and(|record| {
                    let changed = !record.adds_to_trash;
                    record.adds_to_trash = true;
                    changed
                });
            if changed {
                sync_file_action_properties(&mut rust);
            }
            changed
        };
        if changed {
            self.as_mut().notify();
        }
    }

    /// 更新底栏进度，并记下这条消息属于哪个动作；并发动作收尾时只能清自己的。
    pub(crate) fn set_file_action_message(mut self: Pin<&mut Self>, action_id: u64, text: &str) {
        {
            let mut rust = self.as_mut().rust_mut();
            if let Some(record) = rust.file_action_records.get_mut(&action_id) {
                record.message = text.to_owned();
            }
            rust.status_message = QString::from(text);
        }
        self.as_mut().notify();
    }

    pub(crate) fn owns_file_action_message(&self, action_id: u64) -> bool {
        let rust = self.rust();
        rust.file_action_records
            .get(&action_id)
            .is_some_and(|record| {
                !record.message.is_empty() && rust.status_message.to_string() == record.message
            })
    }
}

impl KichiRust {
    /// 某类页面当前是否有写操作在途；刷新只等自己这一类，不被别页慢任务拖住。
    pub(crate) fn has_file_action_domain(&self, domain: FileActionDomain) -> bool {
        self.file_action_records
            .values()
            .any(|record| record.domain == domain)
    }

    pub(crate) fn has_trash_add_in_flight(&self) -> bool {
        self.file_action_records
            .values()
            .any(|record| record.adds_to_trash)
    }

    pub(crate) fn has_trash_write(&self) -> bool {
        self.has_file_action_domain(FileActionDomain::Trash) || self.has_trash_add_in_flight()
    }
}

pub(crate) fn sync_file_action_properties(rust: &mut KichiRust) {
    rust.file_action_running = !rust.file_action_records.is_empty();
    rust.trash_action_running = rust.has_trash_write();
    rust.trash_clearing = rust
        .trash_actions
        .values()
        .any(|state| state.kind == super::trash::TrashActionKind::Clear);
    let mut ids: Vec<&str> = rust.file_action_locks.iter().map(String::as_str).collect();
    ids.sort_unstable();
    let mut list = QStringList::default();
    for id in ids {
        list.append(QString::from(id));
    }
    rust.file_action_ids = list;
}

/// 退出登录 / 重置会话时丢掉所有写动作和锁。
pub(crate) fn reset_file_action_state(rust: &mut KichiRust) {
    rust.file_action_records.clear();
    rust.file_action_locks.clear();
    rust.file_mutations.clear();
    sync_file_action_properties(rust);
}

fn unique_ids(ids: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut result = Vec::with_capacity(ids.len());
    for id in ids {
        let id = id.trim();
        if !id.is_empty() && seen.insert(id.to_owned()) {
            result.push(id.to_owned());
        }
    }
    result
}

fn string_list(ids: &[String]) -> QStringList {
    let mut list = QStringList::default();
    for id in ids {
        list.append(QString::from(id.as_str()));
    }
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moving_entries_into_trash_counts_as_trash_activity() {
        let mut rust = KichiRust::default();
        rust.file_action_records.insert(
            1,
            FileActionRecord {
                ids: vec!["file-id".to_owned()],
                domain: FileActionDomain::Files,
                message: String::new(),
                adds_to_trash: true,
            },
        );

        sync_file_action_properties(&mut rust);
        assert!(rust.trash_action_running);
        assert!(!rust.trash_clearing);

        rust.file_action_records.clear();
        sync_file_action_properties(&mut rust);
        assert!(!rust.trash_action_running);
    }
}
