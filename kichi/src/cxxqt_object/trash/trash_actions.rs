//! 回收站写操作：分批还原 / 彻底删除，以及一次清空。
//!
//! 每个动作只锁自己涉及的条目；不同条目的写操作可以同时在途。整库清空
//! 是唯一的例外：它先清空本地列表，在官方后台任务完成前锁住回收站自己的
//! 刷新 / 再清空，但不影响文件页、资料库或其它页面的写操作。

use std::collections::VecDeque;
use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QString, QStringList};
use kichi_core::TrashEntry;

use super::super::actions::run_in_background;
use super::super::file_action_lock::{sync_file_action_properties, FileActionDomain};
use super::super::qobject;
use crate::session::tree_index::Change;
use crate::session::Failure;

/// core 的批量写接口上限；批量还原 / 彻底删除按这个大小分片。
const TRASH_BATCH_SIZE: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TrashActionKind {
    Restore,
    Delete,
    Clear,
}

impl TrashActionKind {
    fn action_name(self) -> &'static str {
        match self {
            Self::Restore => "untrash",
            Self::Delete => "trashDelete",
            Self::Clear => "trashClear",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Restore => "还原",
            Self::Delete => "彻底删除",
            Self::Clear => "清空回收站",
        }
    }

    pub(crate) fn progress(self, done: usize, total: usize) -> String {
        match self {
            Self::Restore => format!("正在还原……已处理 {done}/{total} 项"),
            Self::Delete => format!("正在彻底删除……已处理 {done}/{total} 项"),
            Self::Clear => "正在清空回收站……".to_owned(),
        }
    }

    pub(crate) fn success(self, total: usize) -> String {
        match self {
            Self::Restore => format!("已还原 {total} 项"),
            Self::Delete => format!("已彻底删除 {total} 项"),
            Self::Clear => format!("回收站已清空（共 {total} 项）"),
        }
    }
}

/// 分批操作的剩余项与进度（只在 Kichi 主线程读写）。
pub(crate) struct TrashActionState {
    pub(crate) kind: TrashActionKind,
    pub(crate) remaining: VecDeque<String>,
    total: usize,
    completed: usize,
    pub(crate) removed: Vec<TrashEntry>,
}

impl qobject::Kichi {
    /// 从回收站还原选中条目。
    pub fn untrash_entries(mut self: Pin<&mut Self>, ids: QStringList) {
        let ids = clean_ids(ids);
        if ids.is_empty() {
            self.as_mut().set_message("没有选中要还原的条目");
            return;
        }
        if self.trash_clearing {
            self.as_mut().set_message("回收站正在清空，请稍后再还原");
            return;
        }
        let Some(action_id) = self
            .as_mut()
            .begin_file_action(&ids, FileActionDomain::Trash)
        else {
            self.as_mut()
                .set_message("相关条目正在处理中，请稍后再还原");
            return;
        };
        self.as_mut()
            .start_trash_action(action_id, TrashActionKind::Restore, ids, Vec::new());
    }

    /// 彻底删除选中条目。不可恢复，QML 负责先弹确认框。
    pub fn delete_trash_entries(mut self: Pin<&mut Self>, ids: QStringList) -> bool {
        let ids = clean_ids(ids);
        if ids.is_empty() {
            self.as_mut().set_message("没有选中要彻底删除的条目");
            return false;
        }
        if self.trash_clearing {
            self.as_mut()
                .set_message("回收站正在清空，请稍后再彻底删除");
            return false;
        }
        let Some(action_id) = self
            .as_mut()
            .begin_file_action(&ids, FileActionDomain::Trash)
        else {
            self.as_mut()
                .set_message("相关条目正在处理中，请稍后再彻底删除");
            return false;
        };
        let removed = self.as_mut().remove_trash_entries_locally(&ids);
        self.as_mut()
            .start_trash_action(action_id, TrashActionKind::Delete, ids, removed);
        true
    }

    /// 调用官方 `trash:empty` 一次性清空当前账号的回收站。
    ///
    /// 请求立即交给后台线程，等待官方任务完成后再回报
    /// `fileActionDone("trashClear", …)`；QML 不需要同步等待。
    pub fn clear_trash(mut self: Pin<&mut Self>) -> bool {
        if self
            .as_ref()
            .rust()
            .has_file_action_domain(FileActionDomain::Trash)
        {
            self.as_mut().set_message("回收站正在处理中，请稍后再清空");
            return false;
        }
        if self.as_ref().rust().has_trash_add_in_flight() {
            self.as_mut()
                .set_message("有文件正在移入回收站，请稍后再清空");
            return false;
        }
        if !self.trash_loaded {
            self.as_mut().set_message("回收站还没读取完成，请稍后再试");
            return false;
        }
        let total = self.trash_list.len();
        if total == 0 {
            self.as_mut().set_message("回收站已经是空的");
            return false;
        }
        let Some(action_id) = self
            .as_mut()
            .begin_file_action(&[], FileActionDomain::Trash)
        else {
            self.as_mut().set_message("回收站正在处理中，请稍后再清空");
            return false;
        };
        let removed = self.as_mut().clear_trash_locally();

        {
            let mut rust = self.as_mut().rust_mut();
            rust.trash_actions.insert(
                action_id,
                TrashActionState {
                    kind: TrashActionKind::Clear,
                    remaining: VecDeque::new(),
                    total,
                    completed: 0,
                    removed,
                },
            );
            sync_file_action_properties(&mut rust);
        }
        self.as_mut().notify();
        self.as_mut()
            .set_file_action_message(action_id, &TrashActionKind::Clear.progress(0, total));

        let session = self.backend.session.clone();
        run_in_background(
            move || {
                session.clear_trash()?;
                session.write_index(|index| index.apply_change(Change::RemoveAllTrashed));
                Ok(())
            },
            move |mut qichi, result| {
                let current = qichi
                    .as_ref()
                    .rust()
                    .trash_actions
                    .get(&action_id)
                    .is_some_and(|state| {
                        state.kind == TrashActionKind::Clear && state.total == total
                    });
                if !current {
                    return;
                }
                match result {
                    Ok(()) => qichi.as_mut().finish_trash_success(
                        action_id,
                        TrashActionKind::Clear,
                        total,
                    ),
                    Err(err) => {
                        qichi.as_mut().finish_trash_failure(
                            action_id,
                            TrashActionKind::Clear,
                            0,
                            total,
                            &[],
                            err,
                        );
                    }
                }
            },
        );
        true
    }

    fn start_trash_action(
        mut self: Pin<&mut Self>,
        action_id: u64,
        kind: TrashActionKind,
        ids: Vec<String>,
        removed: Vec<TrashEntry>,
    ) {
        let total = ids.len();
        {
            let mut rust = self.as_mut().rust_mut();
            rust.trash_actions.insert(
                action_id,
                TrashActionState {
                    kind,
                    remaining: ids.clone().into(),
                    total,
                    completed: 0,
                    removed,
                },
            );
            sync_file_action_properties(&mut rust);
        }
        self.as_mut().notify();
        self.as_mut().run_next_trash_chunk(action_id);
    }

    fn run_next_trash_chunk(mut self: Pin<&mut Self>, action_id: u64) {
        let prepared = {
            let mut rust = self.as_mut().rust_mut();
            let Some(state) = rust.trash_actions.get_mut(&action_id) else {
                return;
            };
            let take = state.remaining.len().min(TRASH_BATCH_SIZE);
            let chunk: Vec<String> = state.remaining.drain(..take).collect();
            if chunk.is_empty() {
                rust.trash_actions.remove(&action_id);
                sync_file_action_properties(&mut rust);
                None
            } else {
                let completed = state.completed;
                let next_done = state.completed + chunk.len();
                Some((state.kind, chunk, state.total, completed, next_done))
            }
        };
        let Some((kind, chunk, total, completed, next_done)) = prepared else {
            self.as_mut().release_file_action(action_id);
            return;
        };
        self.as_mut()
            .set_file_action_message(action_id, &kind.progress(completed, total));

        let session = self.backend.session.clone();
        let failed_ids = chunk.clone();
        run_in_background(
            move || {
                let refs: Vec<&str> = chunk.iter().map(String::as_str).collect();
                match kind {
                    TrashActionKind::Restore => {
                        session.untrash_entries(&refs)?;
                        session.write_index(|index| index.apply_change(Change::Restore(&chunk)));
                    }
                    TrashActionKind::Delete | TrashActionKind::Clear => {
                        session.delete_entries_permanently(&refs)?;
                        session.write_index(|index| index.apply_change(Change::Remove(&chunk)));
                    }
                }
                Ok(())
            },
            move |mut qichi, result| {
                let current = qichi
                    .as_ref()
                    .rust()
                    .trash_actions
                    .get(&action_id)
                    .is_some_and(|state| state.kind == kind && state.total == total);
                if !current {
                    return;
                }
                match result {
                    Ok(()) => {
                        let finished = {
                            let mut rust = qichi.as_mut().rust_mut();
                            let Some(state) = rust.trash_actions.get_mut(&action_id) else {
                                return;
                            };
                            state.completed = next_done;
                            state.remaining.is_empty()
                        };
                        if finished {
                            qichi.as_mut().finish_trash_success(action_id, kind, total);
                        } else {
                            qichi.as_mut().run_next_trash_chunk(action_id);
                        }
                    }
                    Err(err) => {
                        let completed = qichi
                            .as_ref()
                            .rust()
                            .trash_actions
                            .get(&action_id)
                            .map(|state| state.completed)
                            .unwrap_or(0);
                        qichi.as_mut().finish_trash_failure(
                            action_id,
                            kind,
                            completed,
                            total,
                            &failed_ids,
                            err,
                        );
                    }
                }
            },
        );
    }

    fn finish_trash_success(
        mut self: Pin<&mut Self>,
        action_id: u64,
        kind: TrashActionKind,
        total: usize,
    ) {
        let message = kind.success(total);
        let owns_message = self.as_ref().owns_file_action_message(action_id);
        let ids = {
            let mut rust = self.as_mut().rust_mut();
            rust.trash_actions.remove(&action_id);
            sync_file_action_properties(&mut rust);
            if owns_message {
                rust.status_message = QString::default();
            }
            if kind == TrashActionKind::Restore {
                rust.files_stale = true;
            }
            rust.file_action_records
                .get(&action_id)
                .map(|record| record.ids.clone())
                .unwrap_or_default()
        };
        if kind == TrashActionKind::Restore {
            self.as_mut().invalidate_file_caches();
        }
        let ids = ids_from(&ids);
        self.as_mut().release_file_action(action_id);
        self.as_mut().file_action_done(
            QString::from(kind.action_name()),
            true,
            QString::from(message.as_str()),
            ids,
        );
        self.as_mut().refresh_trash_with_status(None, false);
        self.as_mut().resume_pending_trash_refresh();
    }

    fn finish_trash_failure(
        mut self: Pin<&mut Self>,
        action_id: u64,
        kind: TrashActionKind,
        completed: usize,
        total: usize,
        failed_ids: &[String],
        failure: Failure,
    ) {
        let reason = failure.to_string();
        let restored = self.as_mut().take_entries_to_restore(action_id, failed_ids);
        self.as_mut().publish_restored_entries(restored);
        let ids = {
            let mut rust = self.as_mut().rust_mut();
            rust.trash_actions.remove(&action_id);
            sync_file_action_properties(&mut rust);
            if kind == TrashActionKind::Restore && completed > 0 {
                rust.files_stale = true;
            }
            rust.file_action_records
                .get(&action_id)
                .map(|record| record.ids.clone())
                .unwrap_or_default()
        };
        if kind == TrashActionKind::Restore && completed > 0 {
            self.as_mut().invalidate_file_caches();
        }
        self.as_mut().on_failure(failure);
        let message = match kind {
            TrashActionKind::Restore if completed > 0 => {
                format!(
                    "{}中断：已处理 {completed}/{total} 项；{reason}",
                    kind.label()
                )
            }
            TrashActionKind::Restore => format!("{}失败：{reason}", kind.label()),
            TrashActionKind::Clear => format!("{}失败，列表已恢复：{reason}", kind.label()),
            TrashActionKind::Delete if completed > 0 => format!(
                "{}中断：已删除 {completed}/{total} 项，未完成的已恢复；{reason}",
                kind.label()
            ),
            TrashActionKind::Delete => {
                format!("{}失败，列表已恢复：{reason}", kind.label())
            }
        };
        self.as_mut().set_message(&message);
        let ids = ids_from(&ids);
        self.as_mut().release_file_action(action_id);
        self.as_mut().file_action_done(
            QString::from(kind.action_name()),
            false,
            QString::from(message.as_str()),
            ids,
        );
        if kind == TrashActionKind::Restore && completed > 0 {
            self.as_mut()
                .refresh_trash_with_status(Some(message), false);
        }
        self.as_mut().resume_pending_trash_refresh();
    }
}

fn ids_from(ids: &[String]) -> QStringList {
    let mut list = QStringList::default();
    for id in ids {
        list.append(QString::from(id.as_str()));
    }
    list
}

fn clean_ids(ids: QStringList) -> Vec<String> {
    ids.iter()
        .map(|id| id.to_string().trim().to_owned())
        .filter(|id| !id.is_empty())
        .collect()
}
