//! 离线任务入口：提交、删除、清理与播放请求。

use std::path::PathBuf;
use std::pin::Pin;

use cxx_qt_lib::QString;
use kichi_core::{OfflineTask, TaskPhase};

use crate::cxxqt_object::actions::run_in_background;
use crate::cxxqt_object::qobject;
use crate::cxxqt_object::tasks_submit::{
    parse_input, retry_url, submit, submit_torrents, summary, torrent_summary,
};
use crate::session::Failure;

use super::*;

impl qobject::Kichi {
    /// 提交离线下载（磁力 / ed2k / URL），支持一行一个链接。
    pub fn add_task(mut self: Pin<&mut Self>, text: QString) {
        self.as_mut().add_task_to_destination(text, None);
    }

    /// 提交离线下载并保存到指定云端目录；空目录 id 表示根目录。
    pub fn add_task_to_folder(mut self: Pin<&mut Self>, text: QString, folder_id: QString) {
        self.as_mut()
            .add_task_to_destination(text, Some(folder_id.to_string()));
    }

    fn add_task_to_destination(mut self: Pin<&mut Self>, text: QString, parent_id: Option<String>) {
        let parsed = parse_input(&text.to_string());
        if parsed.urls.is_empty() {
            let message = match parsed.invalid_lines.first() {
                Some(line) => format!("第 {line} 行不是有效的磁力 / ed2k / HTTP 链接"),
                None => "没有可提交的链接：每行需要一个磁力 / ed2k / HTTP 地址".to_owned(),
            };
            self.as_mut().set_message(&message);
            return;
        }
        let urls = parsed.urls;
        let ignored = parsed.invalid_lines.len();
        let count = urls.len();
        let session = self.backend.session.clone();
        let destination = parent_id.clone();
        let my_epoch = self.tasks_epoch;
        self.as_mut()
            .set_message(&format!("正在提交 {count} 条离线任务……"));
        run_in_background(
            move || Ok::<_, Failure>(submit(&session, &urls, parent_id.as_deref())),
            move |mut qichi, result| {
                if qichi.as_ref().rust().tasks_epoch != my_epoch {
                    return;
                }
                let outcome = match result {
                    Ok(outcome) => outcome,
                    Err(err) => {
                        qichi.as_mut().on_tasks_failure(err);
                        return;
                    }
                };
                if let Some(failure) = outcome.auth_failure {
                    qichi.as_mut().on_tasks_failure(failure);
                    return;
                }
                if outcome.errors.is_empty() && !outcome.stopped_early && ignored == 0 {
                    qichi.as_mut().clear_status_message();
                } else {
                    let mut message = summary(&outcome);
                    if ignored > 0 {
                        message.push_str(&format!("；另有 {ignored} 行无法识别，已跳过"));
                    }
                    qichi.as_mut().set_message(&message);
                }
                qichi
                    .as_mut()
                    .remember_task_destinations(&outcome.created, destination.as_deref());
                qichi.as_mut().show_submitted_tasks(outcome.created);
                qichi.as_mut().refresh_tasks_force();
            },
        );
    }

    /// 文件页拖入 `.torrent`：本地解析后按拖入时目录提交离线任务。
    pub(crate) fn submit_torrent_files(
        mut self: Pin<&mut Self>,
        paths: Vec<PathBuf>,
        parent_id: Option<String>,
    ) {
        if paths.is_empty() {
            return;
        }
        let count = paths.len();
        let session = self.backend.session.clone();
        let destination = parent_id.clone();
        let my_epoch = self.tasks_epoch;
        self.as_mut()
            .set_message(&format!("正在解析并提交 {count} 个 .torrent 文件……"));
        run_in_background(
            move || Ok::<_, Failure>(submit_torrents(&session, &paths, parent_id.as_deref())),
            move |mut qichi, result| {
                if qichi.as_ref().rust().tasks_epoch != my_epoch {
                    return;
                }
                let outcome = match result {
                    Ok(outcome) => outcome,
                    Err(err) => {
                        qichi.as_mut().on_tasks_failure(err);
                        return;
                    }
                };
                if let Some(failure) = outcome.auth_failure {
                    qichi.as_mut().on_tasks_failure(failure);
                    return;
                }
                if outcome.errors.is_empty() && !outcome.stopped_early {
                    qichi.as_mut().clear_status_message();
                } else {
                    qichi.as_mut().set_message(&torrent_summary(&outcome));
                }
                qichi
                    .as_mut()
                    .remember_task_destinations(&outcome.created, destination.as_deref());
                qichi.as_mut().show_submitted_tasks(outcome.created);
                qichi.as_mut().refresh_tasks_force();
            },
        );
    }

    /// 重新提交失败 / 状态未知的离线任务。
    ///
    /// 来源链接取任务响应里的 `params.url`；目标目录优先沿用本次会话提交时记下的
    /// 目录，其次是任务资源下发的父目录，都确定不了时才回到任务页默认的 My Pack。
    pub fn retry_task(mut self: Pin<&mut Self>, id: QString) {
        let id = id.to_string();
        let task = self.task_list.iter().find(|task| task.id == id).cloned();
        let Some(task) = task else {
            self.as_mut()
                .set_message("任务不在当前列表中，请刷新后重试");
            return;
        };
        if !is_retryable(&task) {
            self.as_mut()
                .set_message("只有失败或状态未知的任务可以重新提交");
            return;
        }
        let Some(url) = retry_url(task.source_url.as_deref()) else {
            self.as_mut()
                .set_message("该任务没有可用的来源链接，无法重新提交");
            return;
        };
        let destination = self
            .task_destinations
            .get(&id)
            .cloned()
            .or_else(|| task.output_parent_id.clone());

        let session = self.backend.session.clone();
        let my_epoch = self.tasks_epoch;
        let request_destination = destination.clone();
        self.as_mut().set_message("正在重新提交离线任务……");
        run_in_background(
            move || {
                let urls = [url];
                Ok::<_, Failure>(submit(&session, &urls, request_destination.as_deref()))
            },
            move |mut qichi, result| {
                if qichi.as_ref().rust().tasks_epoch != my_epoch {
                    return;
                }
                let outcome = match result {
                    Ok(outcome) => outcome,
                    Err(err) => {
                        qichi.as_mut().on_tasks_failure(err);
                        return;
                    }
                };
                if let Some(failure) = outcome.auth_failure {
                    qichi.as_mut().on_tasks_failure(failure);
                    return;
                }
                if outcome.errors.is_empty() && !outcome.stopped_early {
                    qichi.as_mut().set_message("已重新提交离线任务");
                } else {
                    qichi.as_mut().set_message(&summary(&outcome));
                }
                qichi
                    .as_mut()
                    .remember_task_destinations(&outcome.created, destination.as_deref());
                qichi.as_mut().show_submitted_tasks(outcome.created);
                qichi.as_mut().refresh_tasks_force();
            },
        );
    }

    /// 删除任务记录（云端文件保留）。界面侧负责二次确认。
    pub fn delete_task(mut self: Pin<&mut Self>, id: QString) {
        if self.tasks_clearing {
            self.as_mut().set_message("正在清理已结束任务，请稍候");
            return;
        }
        let id = id.to_string();
        if id.trim().is_empty() {
            return;
        }
        let session = self.backend.session.clone();
        let my_epoch = self.tasks_epoch;
        let id_for_request = id.clone();
        self.as_mut().set_message("正在删除任务……");
        run_in_background(
            move || session.delete_offline_task(&id_for_request),
            move |mut qichi, result| {
                if qichi.as_ref().rust().tasks_epoch != my_epoch {
                    return;
                }
                match result {
                    Ok(()) => {
                        qichi.as_mut().rust_mut().task_destinations.remove(&id);
                        qichi.as_mut().set_message("任务已删除（云端文件保留）");
                        qichi.as_mut().refresh_tasks_force();
                    }
                    Err(err) => qichi.as_mut().on_tasks_failure(err),
                }
            },
        );
    }

    /// 清理所有已完成、失败和产物已删除的任务记录；云端文件保留。
    pub fn clear_finished_tasks(mut self: Pin<&mut Self>) {
        if self.tasks_clearing {
            return;
        }
        let ids = finished_task_ids(&self.task_list);
        if ids.is_empty() {
            self.as_mut().set_message("没有已结束任务可清理");
            return;
        }

        let total = ids.len();
        let session = self.backend.session.clone();
        let my_epoch = self.tasks_epoch;
        self.as_mut().rust_mut().tasks_clearing = true;
        self.as_mut()
            .set_message(&format!("正在清理 {total} 条已结束任务……"));
        run_in_background(
            move || Ok::<_, Failure>(session.delete_offline_tasks(&ids)),
            move |mut qichi, result| {
                if qichi.as_ref().rust().tasks_epoch != my_epoch {
                    return;
                }
                qichi.as_mut().rust_mut().tasks_clearing = false;

                let refresh = match result {
                    Ok(outcome) => {
                        let result_total = outcome.total;
                        let deleted = outcome.deleted;
                        if let Some(failure) = outcome.failure {
                            let reason = failure.to_string();
                            if matches!(&failure, Failure::Auth(_)) {
                                qichi.as_mut().on_tasks_failure(failure);
                                false
                            } else {
                                let message = if deleted > 0 {
                                    format!(
                                        "清理已结束任务中断：已清理 {deleted}/{result_total} 条；{reason}"
                                    )
                                } else {
                                    format!("清理已结束任务失败：{reason}")
                                };
                                qichi.as_mut().set_message(&message);
                                true
                            }
                        } else {
                            qichi.as_mut().set_message(&format!(
                                "已清理 {deleted} 条已结束任务（云端文件保留）"
                            ));
                            true
                        }
                    }
                    Err(err) => {
                        let auth_failure = matches!(&err, Failure::Auth(_));
                        qichi.as_mut().on_tasks_failure(err);
                        !auth_failure
                    }
                };
                if refresh {
                    qichi.as_mut().refresh_tasks_force();
                }
            },
        );
    }

    /// 播放已完成的离线任务产物。`id` 是产物文件 id，不是任务记录 id。
    pub fn request_task_play(mut self: Pin<&mut Self>, id: QString) {
        let id = id.to_string();
        let task = self
            .task_list
            .iter()
            .find(|task| {
                task.file_id.as_deref() == Some(id.as_str()) && task.phase == TaskPhase::Completed
            })
            .cloned();
        let Some(task) = task else {
            self.as_mut()
                .set_message("该离线任务已完成产物不在当前列表中，请刷新后重试");
            return;
        };
        if !task.media {
            self.as_mut()
                .set_message("该离线任务产物不是视频或音频，无法播放");
            return;
        }
        self.as_mut().request_play_named(&id, &task.name);
    }
}

pub(crate) fn finished_task_ids(tasks: &[OfflineTask]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut ids = Vec::new();
    for task in tasks {
        if !task.status().is_settled() {
            continue;
        }
        let id = task.id.trim();
        if !id.is_empty() && seen.insert(id) {
            ids.push(id.to_owned());
        }
    }
    ids
}
