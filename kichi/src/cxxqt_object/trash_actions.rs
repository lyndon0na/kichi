//! 回收站写操作：分批还原 / 彻底删除，以及一次清空。

use std::collections::VecDeque;
use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QString, QStringList};

use super::super::actions::run_in_background;
use super::super::qobject;
use crate::session::Failure;

/// core 的批量写接口上限；批量还原 / 彻底删除按这个大小分片。
const TRASH_BATCH_SIZE: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TrashActionKind {
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

    pub(super) fn progress(self, done: usize, total: usize) -> String {
        match self {
            Self::Restore => format!("正在还原……已处理 {done}/{total} 项"),
            Self::Delete => format!("正在彻底删除……已处理 {done}/{total} 项"),
            Self::Clear => "正在清空回收站……".to_owned(),
        }
    }

    pub(super) fn success(self, total: usize) -> String {
        match self {
            Self::Restore => format!("已还原 {total} 项"),
            Self::Delete => format!("已彻底删除 {total} 项"),
            Self::Clear => format!("回收站已清空，共彻底删除 {total} 项"),
        }
    }
}

/// 分批操作的剩余项与进度（只在 Kichi 主线程读写）。
pub(crate) struct TrashActionState {
    kind: TrashActionKind,
    remaining: VecDeque<String>,
    total: usize,
    completed: usize,
}

impl qobject::Kichi {
    /// 从回收站还原选中条目。
    pub fn untrash_entries(mut self: Pin<&mut Self>, ids: QStringList) {
        let ids = clean_ids(ids);
        if ids.is_empty() {
            self.as_mut().set_message("没有选中要还原的条目", false);
            return;
        }
        self.as_mut()
            .start_trash_action(TrashActionKind::Restore, ids);
    }

    /// 彻底删除选中条目。不可恢复，QML 负责先弹确认框。
    pub fn delete_trash_entries(mut self: Pin<&mut Self>, ids: QStringList) {
        let ids = clean_ids(ids);
        if ids.is_empty() {
            self.as_mut().set_message("没有选中要彻底删除的条目", false);
            return;
        }
        self.as_mut()
            .start_trash_action(TrashActionKind::Delete, ids);
    }

    /// 调用官方 `trash:empty` 一次性清空当前账号的回收站。
    pub fn clear_trash(mut self: Pin<&mut Self>) {
        if !self.trash_loaded {
            self.as_mut()
                .set_message("回收站还没读取完成，请稍后再试", false);
            return;
        }
        let total = self.trash_list.len();
        if total == 0 {
            self.as_mut().set_message("回收站已经是空的", false);
            return;
        }
        if !self.as_mut().begin_file_action() {
            return;
        }

        {
            let mut rust = self.as_mut().rust_mut();
            rust.trash_action = Some(TrashActionState {
                kind: TrashActionKind::Clear,
                remaining: VecDeque::new(),
                total,
                completed: 0,
            });
        }
        self.as_mut()
            .set_message(&TrashActionKind::Clear.progress(0, total), true);

        let session = self.backend.session.clone();
        run_in_background(
            move || session.clear_trash(),
            move |mut qichi, result| {
                let current = qichi
                    .as_ref()
                    .rust()
                    .trash_action
                    .as_ref()
                    .is_some_and(|state| {
                        state.kind == TrashActionKind::Clear && state.total == total
                    });
                if !current {
                    return;
                }
                match result {
                    Ok(()) => qichi
                        .as_mut()
                        .finish_trash_success(TrashActionKind::Clear, total),
                    Err(err) => {
                        qichi
                            .as_mut()
                            .finish_trash_failure(TrashActionKind::Clear, 0, total, err);
                    }
                }
            },
        );
    }

    fn start_trash_action(mut self: Pin<&mut Self>, kind: TrashActionKind, ids: Vec<String>) {
        if !self.as_mut().begin_file_action() {
            return;
        }
        let total = ids.len();
        {
            let mut rust = self.as_mut().rust_mut();
            rust.trash_action = Some(TrashActionState {
                kind,
                remaining: ids.into(),
                total,
                completed: 0,
            });
        }
        self.as_mut().run_next_trash_chunk();
    }

    fn run_next_trash_chunk(mut self: Pin<&mut Self>) {
        let (kind, chunk, total, completed, next_done) = {
            let mut rust = self.as_mut().rust_mut();
            let Some(state) = rust.trash_action.as_mut() else {
                return;
            };
            let take = state.remaining.len().min(TRASH_BATCH_SIZE);
            let chunk: Vec<String> = state.remaining.drain(..take).collect();
            if chunk.is_empty() {
                rust.trash_action = None;
                rust.file_action_running = false;
                return;
            }
            let completed = state.completed;
            let next_done = state.completed + chunk.len();
            (state.kind, chunk, state.total, completed, next_done)
        };
        self.as_mut()
            .set_message(&kind.progress(completed, total), true);

        let session = self.backend.session.clone();
        run_in_background(
            move || {
                let refs: Vec<&str> = chunk.iter().map(String::as_str).collect();
                match kind {
                    TrashActionKind::Restore => session.untrash_entries(&refs),
                    TrashActionKind::Delete | TrashActionKind::Clear => {
                        session.delete_entries_permanently(&refs)
                    }
                }
            },
            move |mut qichi, result| {
                let current = qichi
                    .as_ref()
                    .rust()
                    .trash_action
                    .as_ref()
                    .is_some_and(|state| state.kind == kind && state.total == total);
                if !current {
                    return;
                }
                match result {
                    Ok(()) => {
                        let finished = {
                            let mut rust = qichi.as_mut().rust_mut();
                            let Some(state) = rust.trash_action.as_mut() else {
                                return;
                            };
                            state.completed = next_done;
                            state.remaining.is_empty()
                        };
                        if finished {
                            qichi.as_mut().finish_trash_success(kind, total);
                        } else {
                            qichi.as_mut().run_next_trash_chunk();
                        }
                    }
                    Err(err) => {
                        let completed = qichi
                            .as_ref()
                            .rust()
                            .trash_action
                            .as_ref()
                            .map(|state| state.completed)
                            .unwrap_or(0);
                        qichi
                            .as_mut()
                            .finish_trash_failure(kind, completed, total, err);
                    }
                }
            },
        );
    }

    fn finish_trash_success(mut self: Pin<&mut Self>, kind: TrashActionKind, total: usize) {
        let message = kind.success(total);
        {
            let mut rust = self.as_mut().rust_mut();
            rust.trash_action = None;
            rust.file_action_running = false;
            if kind == TrashActionKind::Restore {
                rust.files_stale = true;
            }
        }
        self.as_mut().file_action_done(
            QString::from(kind.action_name()),
            true,
            QString::from(message.as_str()),
        );
        self.as_mut().refresh_trash_with_status(Some(message));
    }

    fn finish_trash_failure(
        mut self: Pin<&mut Self>,
        kind: TrashActionKind,
        completed: usize,
        total: usize,
        failure: Failure,
    ) {
        let reason = failure.to_string();
        {
            let mut rust = self.as_mut().rust_mut();
            rust.trash_action = None;
            rust.file_action_running = false;
            if kind == TrashActionKind::Restore && completed > 0 {
                rust.files_stale = true;
            }
        }
        self.as_mut().on_failure(failure);
        let message = if completed > 0 {
            format!(
                "{}中断：已处理 {completed}/{total} 项；{reason}",
                kind.label()
            )
        } else {
            format!("{}失败：{reason}", kind.label())
        };
        self.as_mut().set_message(&message, false);
        self.as_mut().file_action_done(
            QString::from(kind.action_name()),
            false,
            QString::from(message.as_str()),
        );
        if completed > 0 {
            self.as_mut().refresh_trash_with_status(Some(message));
        }
    }
}

fn clean_ids(ids: QStringList) -> Vec<String> {
    ids.iter()
        .map(|id| id.to_string().trim().to_owned())
        .filter(|id| !id.is_empty())
        .collect()
}
