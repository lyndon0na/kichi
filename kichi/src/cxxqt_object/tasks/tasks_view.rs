//! 任务列表 → QML 字段、通知与列表合并。

use std::collections::{HashMap, HashSet};
use std::pin::Pin;

use cxx_qt_lib::{QMap, QMapPair_QString_QVariant, QString, QVariant};
use kichi_core::{EntryKind, OfflineTask, TaskStatus};

use crate::cxxqt_object::actions::format_size;
use crate::cxxqt_object::notifications;
use crate::cxxqt_object::qobject;
use crate::cxxqt_object::QVariantList;
use crate::session::{page_snapshot, thumbnail_cache};

use super::*;

type QVariantMap = QMap<QMapPair_QString_QVariant>;

impl qobject::Kichi {
    pub(crate) fn remember_task_destinations(
        mut self: Pin<&mut Self>,
        tasks: &[OfflineTask],
        destination: Option<&str>,
    ) {
        let Some(destination) = destination else {
            return;
        };
        let destination = destination.trim().to_owned();
        let mut rust = self.as_mut().rust_mut();
        for task in tasks {
            let id = task.id.trim();
            if !id.is_empty() {
                rust.task_destinations
                    .insert(id.to_owned(), destination.clone());
            }
        }
    }

    /// 新建接口已经返回完整任务：先并入当前列表，侧栏徽标不必等下一次 GET。
    pub(crate) fn show_submitted_tasks(mut self: Pin<&mut Self>, submitted: Vec<OfflineTask>) {
        if submitted.is_empty() {
            return;
        }
        let tasks = merge_submitted_tasks(self.task_list.clone(), submitted);
        self.as_mut().show_tasks(tasks, false);
    }

    /// 把任务列表写回属性；`announce` 决定是否对「刚结束」的任务发系统通知。
    pub(crate) fn show_tasks(
        mut self: Pin<&mut Self>,
        mut tasks: Vec<OfflineTask>,
        announce: bool,
    ) {
        sort_tasks_newest_first(&mut tasks);
        let session = self.backend.session.clone();

        let (finished_now, completed_folders) = {
            // 先在 rust_mut 里完成「谁刚结束」的判定，再统一写属性
            let mut rust = self.as_mut().rust_mut();
            let mut fresh = Vec::new();
            for task in &tasks {
                let status = task.status();
                let was_running = rust.known_running.contains_key(task.id.as_str());
                if was_running
                    && announce
                    && status.is_settled()
                    && rust.notified.insert(task.id.clone())
                {
                    fresh.push(task.clone());
                }
                if status.is_settled() {
                    rust.known_running.remove(task.id.as_str());
                } else {
                    rust.known_running.insert(task.id.clone(), true);
                }
            }
            let completed_folders = completed_task_folders(&fresh, &rust.task_destinations);
            for task in &tasks {
                if task.status().is_settled() {
                    rust.task_destinations.remove(&task.id);
                }
            }
            (fresh, completed_folders)
        };

        let mut list = QVariantList::default();
        for task in &tasks {
            list.append(QVariant::from(&task_map(task)));
        }
        let next_wait = next_interval(&tasks);
        let notify;
        {
            let mut rust = self.as_mut().rust_mut();
            let changed = rust.task_list != tasks;
            let visible = rust.tasks_loading;
            if changed {
                // 轮询 / 提交 / 删除产生的新列表写回快照，重启后直接可用。
                page_snapshot::save(&session, page_snapshot::OFFLINE_TASKS, &tasks);
                rust.tasks = list;
                rust.task_list = tasks;
            }
            rust.tasks_loading = false;
            rust.tasks_request_running = false;
            rust.tasks_loaded = true;
            rust.task_refresh_at = Some(Instant::now());
            notify = changed || visible;
        }
        schedule_poll_wait(next_wait);
        if notify {
            self.as_mut().notify();
        }

        for folder_id in completed_folders {
            self.as_mut().mark_file_dir_stale(&folder_id);
            self.as_mut()
                .offline_task_folder_changed(QString::from(folder_id.as_str()));
        }

        let notify_enabled = self.notify_enabled;
        for task in finished_now {
            notify_finished(&task, notify_enabled);
        }
    }
}

pub(crate) fn sort_tasks_newest_first(tasks: &mut [OfflineTask]) {
    tasks.sort_by(|a, b| b.created_at.cmp(&a.created_at));
}

/// 任务完成后应校准的目录；优先用提交时记下的目标，重启后回退到资源父目录。
pub(crate) fn completed_task_folders(
    tasks: &[OfflineTask],
    destinations: &HashMap<String, String>,
) -> HashSet<String> {
    tasks
        .iter()
        .filter(|task| task.status() == TaskStatus::Completed)
        .filter_map(|task| {
            destinations
                .get(&task.id)
                .cloned()
                .or_else(|| task.output_parent_id.clone())
        })
        .collect()
}

/// 把新建接口返回的任务按 id 合并进当前快照，避免列表接口稍慢时徽标不出现。
pub(crate) fn merge_submitted_tasks(
    mut current: Vec<OfflineTask>,
    submitted: Vec<OfflineTask>,
) -> Vec<OfflineTask> {
    for task in submitted {
        if let Some(existing) = current.iter_mut().find(|current| current.id == task.id) {
            *existing = task;
        } else {
            current.push(task);
        }
    }
    current
}

/// 桌面通知（走 XDG 门户，不依赖额外守护进程）。
pub(crate) fn notify_finished(task: &OfflineTask, enabled: bool) {
    if !enabled {
        return;
    }
    let (summary, body) = match task.status() {
        TaskStatus::Completed => (
            "离线任务完成",
            format!(
                "{}\n{}",
                task.name,
                if task.file_size > 0 {
                    format_size(task.file_size)
                } else {
                    String::new()
                }
            ),
        ),
        TaskStatus::ProductDeleted => (
            "离线任务产物已删除",
            format!(
                "{}\n{}",
                task.name,
                task.message.as_deref().unwrap_or("云端产物已不存在")
            ),
        ),
        TaskStatus::Failed => (
            "离线任务失败",
            format!(
                "{}\n{}",
                task.name,
                task.message.as_deref().unwrap_or("官方未给出原因")
            ),
        ),
        _ => return,
    };
    notifications::show(summary, body.trim_end(), "folder-download");
}

pub(crate) fn task_map(task: &OfflineTask) -> QVariantMap {
    let text = |value: &str| QVariant::from(&QString::from(value));
    let kind = match task.kind {
        Some(EntryKind::Folder) => "folder",
        Some(EntryKind::File) => "file",
        None => "",
    };
    let status = task.status();
    let (phase, phase_text) = match status {
        TaskStatus::Pending => ("pending", "排队中"),
        TaskStatus::Running => ("running", "下载中"),
        TaskStatus::Paused => ("paused", "已暂停"),
        TaskStatus::Completed => ("completed", "已完成"),
        TaskStatus::Failed => ("error", "失败"),
        TaskStatus::ProductDeleted => ("deleted", "产物已删除"),
        TaskStatus::Unknown => ("unknown", "状态未知"),
    };
    let size = if task.file_size > 0 {
        format_size(task.file_size)
    } else {
        "—".to_owned()
    };
    let created = task
        .created_at
        .map(|time| {
            time.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default();
    let detail = task.message.clone().unwrap_or_default();

    let mut map = QVariantMap::default();
    map.insert_clone(&QString::from("id"), &text(&task.id));
    map.insert_clone(&QString::from("name"), &text(&task.name));
    map.insert_clone(&QString::from("kind"), &text(kind));
    map.insert_clone(&QString::from("phase"), &text(phase));
    map.insert_clone(&QString::from("phaseText"), &text(phase_text));
    map.insert_clone(
        &QString::from("progress"),
        &QVariant::from(&i32::from(task.progress)),
    );
    map.insert_clone(
        &QString::from("progressText"),
        &text(&format!("{}%", task.progress)),
    );
    map.insert_clone(&QString::from("sizeText"), &text(&size));
    map.insert_clone(&QString::from("createdText"), &text(&created));
    map.insert_clone(&QString::from("detail"), &text(&detail));
    map.insert_clone(
        &QString::from("fileId"),
        &text(task.file_id.as_deref().unwrap_or_default()),
    );
    map.insert_clone(
        &QString::from("hasThumbnail"),
        &QVariant::from(&has_task_thumbnail(task)),
    );
    map.insert_clone(&QString::from("video"), &QVariant::from(&task.video));
    map.insert_clone(
        &QString::from("playable"),
        &QVariant::from(&(task.file_id.is_some() && task.media)),
    );
    map.insert_clone(
        &QString::from("settled"),
        &QVariant::from(&status.is_settled()),
    );
    map
}

/// 远程地址或本地缓存任一可用即可显示缩略图；快照不保存签名地址，
/// 冷启动只能靠本地缓存先出图。
pub(crate) fn has_task_thumbnail(task: &OfflineTask) -> bool {
    if task.kind != Some(EntryKind::File) {
        return false;
    }
    if !task.thumbnail_url.is_empty() {
        return true;
    }
    let Some(id) = task
        .file_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    else {
        return false;
    };
    (task.video || kichi_core::is_image_name(&task.name)) && thumbnail_cache::find(id).is_some()
}
