//! 离线任务页：列表、提交、删除、轮询与完成通知。
//!
//! 轮询节奏（官方风险提示要求低频，见设计文档）：
//! - 列表里有未完成任务：10 秒一次
//! - 全部已结束（完成/失败）：5 分钟一次
//!
//! 定时循环跑在一条常驻后台线程里；每轮结果照旧经主线程队列由 `pump()` 取回，
//! 不在后台线程碰任何 Qt 对象。退出登录时 `task_epoch` 自增，在途结果作废。

use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QMap, QMapPair_QString_QVariant, QString, QVariant};
use kichi_core::{OfflineTask, TaskPhase};

use super::actions::{format_size, run_in_background};
use super::qobject;
use super::QVariantList;
use crate::session::{has_active, Failure, Session};

type QVariantMap = QMap<QMapPair_QString_QVariant>;

/// 有任务在跑时的轮询间隔。
const POLL_ACTIVE: Duration = Duration::from_secs(10);
/// 全部任务已结束时的轮询间隔（仅为发现新提交/其他设备的新任务）。
const POLL_IDLE: Duration = Duration::from_secs(5 * 60);
/// 后台轮询线程检查「是否该停」的粒度。
const TICK: Duration = Duration::from_secs(1);

/// 轮询线程的全局开关（进程内唯一）。
fn poll_running() -> &'static AtomicBool {
    static FLAG: OnceLock<AtomicBool> = OnceLock::new();
    FLAG.get_or_init(|| AtomicBool::new(false))
}

/// 轮询是否已启动（避免重复起线程）。
fn poll_started() -> &'static AtomicBool {
    static FLAG: OnceLock<AtomicBool> = OnceLock::new();
    FLAG.get_or_init(|| AtomicBool::new(false))
}

/// 登出代数：后台线程与在途结果都用它判断自己是否已过时。
fn epoch() -> &'static AtomicU64 {
    static EPOCH: OnceLock<AtomicU64> = OnceLock::new();
    EPOCH.get_or_init(|| AtomicU64::new(0))
}

/// 下一次该等多久（单测直接测这个纯函数）。
fn next_interval(tasks: &[OfflineTask]) -> Duration {
    if has_active(tasks) {
        POLL_ACTIVE
    } else {
        POLL_IDLE
    }
}

impl qobject::Kichi {
    /// 重新读取任务列表（登录完成、提交成功、删除成功、刷新按钮都走这里）。
    pub fn refresh_tasks(mut self: Pin<&mut Self>) {
        if self.tasks_loading {
            return;
        }
        let session = self.backend.session.clone();
        let my_epoch = {
            let mut rust = self.as_mut().rust_mut();
            rust.tasks_loading = true;
            rust.tasks_epoch
        };
        run_in_background(
            move || session.offline_tasks(),
            move |mut qichi, result| {
                if qichi.as_ref().rust().tasks_epoch != my_epoch {
                    return;
                }
                qichi.as_mut().rust_mut().tasks_loading = false;
                match result {
                    Ok(tasks) => qichi.as_mut().show_tasks(tasks, true),
                    Err(err) => qichi.as_mut().on_tasks_failure(err),
                }
            },
        );
    }

    /// 提交离线下载（磁力 / ed2k / URL）。
    pub fn add_task(mut self: Pin<&mut Self>, url: QString) {
        let url = url.to_string();
        let url = url.trim();
        if url.is_empty() {
            return;
        }
        let url = url.to_owned();
        let session = self.backend.session.clone();
        let my_epoch = self.tasks_epoch;
        self.as_mut().set_message("正在提交离线任务……", true);
        run_in_background(
            move || session.add_offline_task(&url),
            move |mut qichi, result| {
                if qichi.as_ref().rust().tasks_epoch != my_epoch {
                    return;
                }
                match result {
                    Ok(task) => {
                        qichi
                            .as_mut()
                            .set_message(&format!("已提交：{}", task.name), false);
                        qichi.as_mut().clear_task_input();
                        qichi.as_mut().refresh_tasks();
                    }
                    Err(err) => qichi.as_mut().on_tasks_failure(err),
                }
            },
        );
    }

    /// 删除任务记录（云端文件保留）。界面侧负责二次确认。
    pub fn delete_task(mut self: Pin<&mut Self>, id: QString) {
        let id = id.to_string();
        if id.trim().is_empty() {
            return;
        }
        let session = self.backend.session.clone();
        let my_epoch = self.tasks_epoch;
        self.as_mut().set_message("正在删除任务……", true);
        run_in_background(
            move || session.delete_offline_task(&id),
            move |mut qichi, result| {
                if qichi.as_ref().rust().tasks_epoch != my_epoch {
                    return;
                }
                match result {
                    Ok(()) => {
                        qichi
                            .as_mut()
                            .set_message("任务已删除（云端文件保留）", false);
                        qichi.as_mut().refresh_tasks();
                    }
                    Err(err) => qichi.as_mut().on_tasks_failure(err),
                }
            },
        );
    }

    /// 登录成功后启动轮询线程（幂等）。
    pub(crate) fn start_task_polling(&self) {
        if poll_started().swap(true, Ordering::SeqCst) {
            poll_running().store(true, Ordering::SeqCst);
            return;
        }
        poll_running().store(true, Ordering::SeqCst);
        let session = self.backend.session.clone();
        let spawned = std::thread::Builder::new()
            .name("kichi-poll".into())
            .spawn(move || poll_loop(session));
        if let Err(err) = spawned {
            poll_started().store(false, Ordering::SeqCst);
            eprintln!("[kichi] 无法创建任务轮询线程：{err}");
        }
    }

    /// 退出登录：让轮询线程歇下来，并作废所有在途结果。
    pub(crate) fn stop_task_polling(mut self: Pin<&mut Self>) {
        poll_running().store(false, Ordering::SeqCst);
        self.as_mut().reset_tasks();
    }

    /// 退出登录时清空任务页状态。
    pub(crate) fn reset_tasks(mut self: Pin<&mut Self>) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.tasks_epoch = rust.tasks_epoch.wrapping_add(1);
            rust.tasks_loading = false;
            rust.tasks = QVariantList::default();
            rust.task_list.clear();
            rust.notified.clear();
            rust.task_input = QString::default();
        }
        epoch().fetch_add(1, Ordering::SeqCst);
        self.as_mut().notify();
    }

    /// 任务页专属的错误出口：不打断登录状态，只提示。
    fn on_tasks_failure(mut self: Pin<&mut Self>, failure: Failure) {
        if matches!(failure, Failure::Auth(_)) {
            // 令牌彻底失效：交回统一的登录失效处理（会切回登录页）
            self.as_mut().on_failure(failure);
            return;
        }
        self.as_mut().set_message(&failure.to_string(), false);
    }

    /// 把任务列表写回属性；`announce` 决定是否对「刚结束」的任务发系统通知。
    pub(crate) fn show_tasks(
        mut self: Pin<&mut Self>,
        mut tasks: Vec<OfflineTask>,
        announce: bool,
    ) {
        // 进行中的排前面，其余按创建时间新→旧
        tasks.sort_by(|a, b| {
            let rank = |t: &OfflineTask| match t.phase {
                TaskPhase::Running => 0,
                TaskPhase::Pending => 1,
                TaskPhase::Paused | TaskPhase::Unknown => 2,
                TaskPhase::Error => 3,
                TaskPhase::Completed => 4,
            };
            rank(a)
                .cmp(&rank(b))
                .then_with(|| b.created_at.cmp(&a.created_at))
        });

        let finished_now = if announce {
            // 先在 rust_mut 里完成「谁刚结束」的判定，再统一写属性
            let mut rust = self.as_mut().rust_mut();
            let mut fresh = Vec::new();
            for task in &tasks {
                let was_running = rust.known_running.contains_key(task.id.as_str());
                if was_running && task.phase.is_settled() && rust.notified.insert(task.id.clone()) {
                    fresh.push(task.clone());
                }
                if task.phase.is_settled() {
                    rust.known_running.remove(task.id.as_str());
                } else {
                    rust.known_running.insert(task.id.clone(), true);
                }
            }
            fresh
        } else {
            Vec::new()
        };

        let mut list = QVariantList::default();
        for task in &tasks {
            list.append(QVariant::from(&task_map(task)));
        }
        let active = tasks.iter().filter(|t| !t.phase.is_settled()).count();
        let total = tasks.len();
        {
            let mut rust = self.as_mut().rust_mut();
            rust.tasks = list;
            rust.task_list = tasks;
            rust.tasks_loading = false;
        }
        let summary = if total == 0 {
            "暂无离线任务".to_owned()
        } else if active == 0 {
            format!("离线任务 {total} 条，均已完成")
        } else {
            format!("离线任务 {total} 条，{active} 条进行中")
        };
        self.as_mut().set_message(&summary, false);
        self.as_mut().notify();

        let notify_enabled = self.notify_enabled;
        for task in finished_now {
            notify_finished(&task, notify_enabled);
        }
    }

    /// 清空输入框（属性 setter 由 cxx-qt 生成，这里换个名字避免撞车）。
    fn clear_task_input(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().task_input = QString::default();
        self.as_mut().notify();
    }
}

/// 后台轮询循环：按间隔拉列表，结果回投到主线程。
///
/// 间隔由「上一轮结果」决定，因此写在一个只被本线程读写的局部变量里；
/// 每 TICK 醒来一次检查是否该退出（登出）。
fn poll_loop(session: Arc<Session>) {
    loop {
        let interval = poll_wait().load(Ordering::SeqCst);
        if !sleep_interruptible(Duration::from_secs(interval)) {
            return;
        }
        let my_epoch = epoch().load(Ordering::SeqCst);
        let session = Arc::clone(&session);
        run_in_background(
            move || session.offline_tasks(),
            move |mut qichi, result| {
                if qichi.as_ref().rust().tasks_epoch != my_epoch {
                    return;
                }
                match result {
                    Ok(tasks) => {
                        // 下一轮的间隔由这一轮结果决定（在主线程算，避免再开共享状态）
                        let next = next_interval(&tasks);
                        poll_wait().store(next.as_secs(), Ordering::SeqCst);
                        qichi.as_mut().show_tasks(tasks, true);
                    }
                    // 轮询失败只在状态栏提示，不打断正在浏览文件的用户
                    Err(err) => qichi
                        .as_mut()
                        .set_message(&format!("任务刷新失败：{err}"), false),
                }
            },
        );
    }
}

/// 下一轮轮询的间隔（秒）；初值 = 活动间隔，保证登录后尽快出数据。
fn poll_wait() -> &'static AtomicU64 {
    static WAIT: OnceLock<AtomicU64> = OnceLock::new();
    WAIT.get_or_init(|| AtomicU64::new(POLL_ACTIVE.as_secs()))
}

/// 睡 `total`，期间每 [`TICK`] 检查一次是否该停；返回 `false` 表示要退出。
fn sleep_interruptible(total: Duration) -> bool {
    let mut slept = Duration::ZERO;
    while slept < total {
        if !poll_running().load(Ordering::SeqCst) {
            return false;
        }
        let step = TICK.min(total - slept);
        std::thread::sleep(step);
        slept += step;
    }
    poll_running().load(Ordering::SeqCst)
}

/// 桌面通知（走 XDG 门户，不依赖额外守护进程）。
fn notify_finished(task: &OfflineTask, enabled: bool) {
    if !enabled {
        return;
    }
    let (summary, body) = match task.phase {
        TaskPhase::Completed => (
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
        TaskPhase::Error => (
            "离线任务失败",
            format!(
                "{}\n{}",
                task.name,
                task.message.as_deref().unwrap_or("官方未给出原因")
            ),
        ),
        _ => return,
    };
    super::notifications::show(summary, body.trim_end(), "folder-download");
}

/// 一条任务 → QML 用的小 map（只带界面要显示的字段）。
fn task_map(task: &OfflineTask) -> QVariantMap {
    let text = |value: &str| QVariant::from(&QString::from(value));
    let (phase, phase_text) = match task.phase {
        TaskPhase::Pending => ("pending", "排队中"),
        TaskPhase::Running => ("running", "下载中"),
        TaskPhase::Paused => ("paused", "已暂停"),
        TaskPhase::Completed => ("completed", "已完成"),
        TaskPhase::Error => ("error", "失败"),
        TaskPhase::Unknown => ("unknown", "状态未知"),
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
                .format("%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default();
    let detail = task.message.clone().unwrap_or_default();

    let mut map = QVariantMap::default();
    map.insert_clone(&QString::from("id"), &text(&task.id));
    map.insert_clone(&QString::from("name"), &text(&task.name));
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
        &QString::from("settled"),
        &QVariant::from(&task.phase.is_settled()),
    );
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str, phase: TaskPhase) -> OfflineTask {
        OfflineTask {
            id: id.to_owned(),
            name: id.to_owned(),
            phase,
            progress: 0,
            message: None,
            created_at: None,
            file_id: None,
            file_size: 0,
        }
    }

    #[test]
    fn interval_depends_on_active_tasks() {
        assert_eq!(next_interval(&[]), POLL_IDLE);
        assert_eq!(
            next_interval(&[task("a", TaskPhase::Completed), task("b", TaskPhase::Error)]),
            POLL_IDLE
        );
        assert_eq!(next_interval(&[task("c", TaskPhase::Running)]), POLL_ACTIVE);
        assert_eq!(next_interval(&[task("d", TaskPhase::Pending)]), POLL_ACTIVE);
        assert_eq!(next_interval(&[task("e", TaskPhase::Paused)]), POLL_ACTIVE);
    }

    #[test]
    fn task_map_exposes_ui_fields() {
        let mut t = task("id1", TaskPhase::Running);
        t.name = "电影.mkv".to_owned();
        t.progress = 42;
        t.file_size = 123_456;
        let map = task_map(&t);
        let get = |key: &str| {
            map.get(&QString::from(key))
                .and_then(|v| v.value::<QString>())
                .map(|s| s.to_string())
                .unwrap_or_default()
        };
        assert_eq!(get("phase"), "running");
        assert_eq!(get("progressText"), "42%");
        assert_eq!(get("name"), "电影.mkv");
        assert_eq!(get("sizeText"), "121 KB");
    }

    #[test]
    fn progress_is_integer_percent() {
        let mut t = task("id1", TaskPhase::Running);
        t.progress = 50;
        let map = task_map(&t);
        let value = map
            .get(&QString::from("progress"))
            .and_then(|v| v.value::<i32>());
        assert_eq!(value, Some(50));
    }

    #[test]
    fn settled_flag_matches_phase() {
        let running = task_map(&task("a", TaskPhase::Running));
        let done = task_map(&task("b", TaskPhase::Completed));
        let read = |map: &QVariantMap| {
            map.get(&QString::from("settled"))
                .and_then(|v| v.value::<bool>())
        };
        assert_eq!(read(&running), Some(false));
        assert_eq!(read(&done), Some(true));
    }
}
