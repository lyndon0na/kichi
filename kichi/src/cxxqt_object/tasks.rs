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
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QMap, QMapPair_QString_QVariant, QString, QVariant};
use kichi_core::{EntryKind, OfflineTask, TaskPhase, TaskStatus};

use super::actions::{format_size, run_in_background};
use super::qobject;
use super::tasks_submit::{parse_urls, submit, summary};
use super::QVariantList;
use crate::session::{has_active, Failure};

type QVariantMap = QMap<QMapPair_QString_QVariant>;

/// 有任务在跑时的轮询间隔。
const POLL_ACTIVE: Duration = Duration::from_secs(10);
/// 全部任务已结束时的轮询间隔（仅为发现新提交/其他设备的新任务）。
const POLL_IDLE: Duration = Duration::from_secs(5 * 60);
/// 后台轮询线程检查「是否该停」的粒度。
const TICK: Duration = Duration::from_secs(1);
/// 手动 / 切页刷新的短冷却：避免来回切页把请求打成连续 GET。
const REFRESH_COOLDOWN: Duration = Duration::from_secs(3);
/// 切回任务页时，后台轮询结果在这么久内可直接复用。
const TASKS_PAGE_TTL: Duration = Duration::from_secs(30);

/// 轮询线程的全局开关（进程内唯一）。
fn poll_running() -> &'static AtomicBool {
    static FLAG: OnceLock<AtomicBool> = OnceLock::new();
    FLAG.get_or_init(|| AtomicBool::new(false))
}

/// 轮询线程的代际：每次启动 / 停止都前进一格，旧线程醒来看到代际变了就退出。
///
/// 不能只记「启动过没有」：登出时旧线程要睡满一个 [`TICK`] 才会发现该退出，
/// 这期间又登录的话新的一次启动会被那个标志挡住；旧线程随后退出，结果再也没人
/// 轮询（任务页要手动刷新到重启为止）。代际让每次登录都起一条属于自己代际的
/// 线程，旧的自己退出。
fn poll_generation() -> &'static AtomicU64 {
    static GENERATION: OnceLock<AtomicU64> = OnceLock::new();
    GENERATION.get_or_init(|| AtomicU64::new(0))
}

/// 下一次该等多久（单测直接测这个纯函数）。
fn next_interval(tasks: &[OfflineTask]) -> Duration {
    if has_active(tasks) {
        POLL_ACTIVE
    } else {
        POLL_IDLE
    }
}

/// 距离上次任务刷新够不够久；`None` 表示本次会话还没拉过。
fn refresh_due(last: Option<Instant>, now: Instant) -> bool {
    match last {
        Some(last) => now.saturating_duration_since(last) >= REFRESH_COOLDOWN,
        None => true,
    }
}

impl qobject::Kichi {
    /// 按需读取任务列表（刷新按钮、进入页面都走这里）。
    pub fn refresh_tasks(mut self: Pin<&mut Self>) {
        self.as_mut().refresh_tasks_inner(false, true);
    }

    /// 后台轮询 / 切页校准：不发可见加载反馈。
    fn refresh_tasks_background(mut self: Pin<&mut Self>) {
        self.as_mut().refresh_tasks_inner(false, false);
    }

    /// 页面重新可见：后台轮询已经维护列表，短时间内不额外发请求。
    pub fn ensure_tasks_fresh(mut self: Pin<&mut Self>) {
        let now = Instant::now();
        if self.tasks_request_running
            || self.tasks_loaded
                && self
                    .task_refresh_at
                    .is_some_and(|at| now.saturating_duration_since(at) < TASKS_PAGE_TTL)
        {
            return;
        }
        self.as_mut().refresh_tasks_inner(false, false);
    }

    /// 提交 / 删除后的强制刷新：绕过短冷却，但仍只允许一轮请求在途。
    fn refresh_tasks_force(mut self: Pin<&mut Self>) {
        self.as_mut().refresh_tasks_inner(true, false);
    }

    fn refresh_tasks_inner(mut self: Pin<&mut Self>, force: bool, manual: bool) {
        let session = self.backend.session.clone();
        let has_cache = self.tasks_loaded;
        if self.tasks_request_running {
            let show_manual = {
                let mut rust = self.as_mut().rust_mut();
                if force {
                    rust.task_refresh_pending = true;
                }
                let show = manual && has_cache && !rust.tasks_loading;
                if show {
                    rust.tasks_loading = true;
                }
                show
            };
            if show_manual {
                self.as_mut().notify();
            }
            return;
        }
        let (my_epoch, visible) = {
            let now = Instant::now();
            let mut rust = self.as_mut().rust_mut();
            if !force && !refresh_due(rust.task_refresh_at, now) {
                return;
            }
            let visible = manual || !has_cache;
            rust.tasks_loading = visible;
            rust.tasks_request_running = true;
            rust.task_refresh_at = Some(now);
            (rust.tasks_epoch, visible)
        };
        // 加载态要立刻显示，不能等网络回来才通知 QML。
        if visible {
            self.as_mut().notify();
        }
        run_in_background(
            move || session.offline_tasks(),
            move |mut qichi, result| {
                if qichi.as_ref().rust().tasks_epoch != my_epoch {
                    return;
                }
                match result {
                    Ok(tasks) => qichi.as_mut().show_tasks(tasks, true),
                    Err(err) => {
                        {
                            let mut rust = qichi.as_mut().rust_mut();
                            rust.tasks_loading = false;
                            rust.tasks_request_running = false;
                            rust.task_refresh_at = Some(Instant::now());
                        }
                        qichi.as_mut().on_tasks_failure(err);
                    }
                }
                // 提交 / 删除恰逢旧列表请求在途时，旧结果可能不含刚写入的任务。
                // 本轮收尾后补一次，避免退回 5 分钟空闲轮询。
                let again = {
                    let mut rust = qichi.as_mut().rust_mut();
                    let pending = rust.task_refresh_pending;
                    rust.task_refresh_pending = false;
                    pending
                };
                if again {
                    qichi.as_mut().refresh_tasks_force();
                }
            },
        );
    }

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
        let urls = parse_urls(&text.to_string());
        if urls.is_empty() {
            return;
        }
        let count = urls.len();
        let session = self.backend.session.clone();
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
                let text = summary(&outcome);
                if let Some(failure) = outcome.auth_failure {
                    qichi.as_mut().on_tasks_failure(failure);
                    return;
                }
                if outcome.errors.is_empty() && !outcome.stopped_early {
                    qichi.as_mut().clear_status_message();
                } else {
                    qichi.as_mut().set_message(&text);
                }
                qichi.as_mut().show_submitted_tasks(outcome.created);
                qichi.as_mut().refresh_tasks_force();
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
        self.as_mut().set_message("正在删除任务……");
        run_in_background(
            move || session.delete_offline_task(&id),
            move |mut qichi, result| {
                if qichi.as_ref().rust().tasks_epoch != my_epoch {
                    return;
                }
                match result {
                    Ok(()) => {
                        qichi.as_mut().set_message("任务已删除（云端文件保留）");
                        qichi.as_mut().refresh_tasks_force();
                    }
                    Err(err) => qichi.as_mut().on_tasks_failure(err),
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

    /// 登录成功后启动轮询线程。
    ///
    /// 每次调用都起一条新代际的线程，上一条（如果还睡着）醒来看到代际变了就退出。
    pub(crate) fn start_task_polling(mut self: Pin<&mut Self>) {
        let generation = poll_generation().fetch_add(1, Ordering::SeqCst) + 1;
        poll_running().store(true, Ordering::SeqCst);
        schedule_poll_wait(POLL_ACTIVE);
        let spawned = std::thread::Builder::new()
            .name("kichi-poll".into())
            .spawn(move || poll_loop(generation));
        if let Err(err) = spawned {
            // 起不来就别让开关停在 true：下一次登录还会再试
            poll_running().store(false, Ordering::SeqCst);
            eprintln!("[kichi] 无法创建任务轮询线程：{err}");
            return;
        }
        // 侧栏徽标也要在启动 / 登录后立即读到云端任务，不能先空等 10 秒。
        self.as_mut().refresh_tasks_background();
    }

    /// 退出登录：让轮询线程歇下来，并作废所有在途结果。
    pub(crate) fn stop_task_polling(mut self: Pin<&mut Self>) {
        // 代际前进一步：旧线程最多等一个 TICK 就发现自己作废、退出
        poll_generation().fetch_add(1, Ordering::SeqCst);
        poll_running().store(false, Ordering::SeqCst);
        self.as_mut().reset_tasks();
    }

    /// 退出登录时清空任务页状态。
    pub(crate) fn reset_tasks(mut self: Pin<&mut Self>) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.tasks_epoch = rust.tasks_epoch.wrapping_add(1);
            rust.tasks_loading = false;
            rust.tasks_request_running = false;
            rust.task_refresh_pending = false;
            rust.tasks_loaded = false;
            rust.task_refresh_at = None;
            rust.tasks = QVariantList::default();
            rust.task_list.clear();
            rust.notified.clear();
        }
        self.as_mut().notify();
    }

    /// 任务页专属的错误出口：不打断登录状态，只提示。
    fn on_tasks_failure(mut self: Pin<&mut Self>, failure: Failure) {
        if matches!(failure, Failure::Auth(_)) {
            // 令牌彻底失效：交回统一的登录失效处理（会切回登录页）
            self.as_mut().on_failure(failure);
            return;
        }
        self.as_mut().set_message(&failure.to_string());
    }

    /// 新建接口已经返回完整任务：先并入当前列表，侧栏徽标不必等下一次 GET。
    fn show_submitted_tasks(mut self: Pin<&mut Self>, submitted: Vec<OfflineTask>) {
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
        // 先按活动程度排序，已结束状态再按失败 / 已删除 / 完成和创建时间排。
        tasks.sort_by(|a, b| {
            let rank = |t: &OfflineTask| match t.status() {
                TaskStatus::Running => 0,
                TaskStatus::Pending => 1,
                TaskStatus::Paused | TaskStatus::Unknown => 2,
                TaskStatus::Failed => 3,
                TaskStatus::ProductDeleted => 4,
                TaskStatus::Completed => 5,
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
                if was_running
                    && task.status().is_settled()
                    && rust.notified.insert(task.id.clone())
                {
                    fresh.push(task.clone());
                }
                if task.status().is_settled() {
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
        let next_wait = next_interval(&tasks);
        let notify;
        {
            let mut rust = self.as_mut().rust_mut();
            let changed = rust.task_list != tasks;
            let visible = rust.tasks_loading;
            if changed {
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

        let notify_enabled = self.notify_enabled;
        for task in finished_now {
            notify_finished(&task, notify_enabled);
        }
    }
}

/// 把新建接口返回的任务按 id 合并进当前快照，避免列表接口稍慢时徽标不出现。
fn merge_submitted_tasks(
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

/// 后台轮询循环：按间隔拉列表，结果回投到主线程。
///
/// 所有请求都回到主线程调用后台刷新入口，与切页 / 手动刷新共用内部单飞保护。
/// 每 TICK 醒来一次检查退出或间隔变化。
fn poll_loop(generation: u64) {
    loop {
        let interval = poll_wait().load(Ordering::SeqCst);
        let revision = poll_wait_revision().load(Ordering::SeqCst);
        match wait_for_poll(Duration::from_secs(interval), generation, revision) {
            PollWait::Stopped => return,
            PollWait::Rescheduled => continue,
            PollWait::Due => {}
        }
        super::actions::enqueue_ui(move |mut qichi| {
            if still_polling(generation) {
                qichi.as_mut().refresh_tasks_background();
            }
        });
    }
}

/// 下一轮轮询的间隔（秒）；成功刷新后会按任务状态重新设置。
fn poll_wait() -> &'static AtomicU64 {
    static WAIT: OnceLock<AtomicU64> = OnceLock::new();
    WAIT.get_or_init(|| AtomicU64::new(POLL_ACTIVE.as_secs()))
}

/// 轮询间隔版本：成功刷新后重置倒计时，手动刷新发现活动任务时也能尽快切回 10 秒。
fn poll_wait_revision() -> &'static AtomicU64 {
    static REVISION: OnceLock<AtomicU64> = OnceLock::new();
    REVISION.get_or_init(|| AtomicU64::new(0))
}

fn schedule_poll_wait(interval: Duration) {
    poll_wait().store(interval.as_secs(), Ordering::SeqCst);
    poll_wait_revision().fetch_add(1, Ordering::SeqCst);
}

#[derive(Debug, Eq, PartialEq)]
enum PollWait {
    Due,
    Rescheduled,
    Stopped,
}

/// 等到下一轮该拉取，或因为间隔变化 / 登出提前结束。
fn wait_for_poll(total: Duration, generation: u64, revision: u64) -> PollWait {
    let mut slept = Duration::ZERO;
    loop {
        if !still_polling(generation) {
            return PollWait::Stopped;
        }
        if poll_wait_revision().load(Ordering::SeqCst) != revision {
            return PollWait::Rescheduled;
        }
        if slept >= total {
            return PollWait::Due;
        }
        let step = TICK.min(total - slept);
        std::thread::sleep(step);
        slept += step;
    }
}

/// 这条轮询线程还算数吗：开关还开着，而且没被新一代顶掉。
fn still_polling(generation: u64) -> bool {
    poll_running().load(Ordering::SeqCst) && poll_generation().load(Ordering::SeqCst) == generation
}

/// 桌面通知（走 XDG 门户，不依赖额外守护进程）。
fn notify_finished(task: &OfflineTask, enabled: bool) {
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
    super::notifications::show(summary, body.trim_end(), "folder-download");
}

/// 一条任务 → QML 用的小 map（只带界面要显示的字段）。
fn task_map(task: &OfflineTask) -> QVariantMap {
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
        &QVariant::from(&(task.kind == Some(EntryKind::File) && !task.thumbnail_url.is_empty())),
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
            kind: None,
            thumbnail_url: String::new(),
            media: false,
            video: false,
        }
    }

    #[test]
    fn a_stale_generation_stops_polling() {
        poll_running().store(true, Ordering::SeqCst);
        let first = poll_generation().fetch_add(1, Ordering::SeqCst) + 1;
        assert!(still_polling(first), "刚启动的代际有效");

        // 登出再登录：代际前进，旧的线程应当立刻醒来退出，不会睡满一个间隔
        let second = poll_generation().fetch_add(1, Ordering::SeqCst) + 1;
        assert!(!still_polling(first), "被新一代顶掉的线程要退出");
        assert!(still_polling(second), "新登录的线程继续轮询");
        let revision = poll_wait_revision().load(Ordering::SeqCst);
        assert!(
            matches!(
                wait_for_poll(Duration::from_secs(5), first, revision),
                PollWait::Stopped
            ),
            "旧代际不睡满"
        );

        // 登出：开关关掉，谁都不再算数
        poll_running().store(false, Ordering::SeqCst);
        assert!(!still_polling(second));
    }

    #[test]
    fn interval_depends_on_active_tasks() {
        assert_eq!(next_interval(&[]), POLL_IDLE);
        let mut deleted = task("deleted", TaskPhase::Error);
        deleted.message = Some("File deleted".into());
        assert_eq!(
            next_interval(&[
                task("a", TaskPhase::Completed),
                task("b", TaskPhase::Error),
                deleted,
            ]),
            POLL_IDLE
        );
        assert_eq!(next_interval(&[task("c", TaskPhase::Running)]), POLL_ACTIVE);
        assert_eq!(next_interval(&[task("d", TaskPhase::Pending)]), POLL_ACTIVE);
        assert_eq!(next_interval(&[task("e", TaskPhase::Paused)]), POLL_ACTIVE);
    }

    #[test]
    fn submitted_tasks_merge_by_id_without_duplicates() {
        let mut updated = task("same", TaskPhase::Running);
        updated.progress = 30;
        let appended = task("new", TaskPhase::Pending);
        let merged = merge_submitted_tasks(
            vec![
                task("old", TaskPhase::Completed),
                task("same", TaskPhase::Pending),
            ],
            vec![updated, appended],
        );

        assert_eq!(merged.len(), 3);
        assert_eq!(
            merged
                .iter()
                .find(|task| task.id == "same")
                .map(|task| task.progress),
            Some(30)
        );
        assert!(merged.iter().any(|task| task.id == "new"));
    }

    #[test]
    fn manual_refresh_has_a_short_cooldown() {
        let now = Instant::now();
        assert!(refresh_due(None, now), "首次刷新不受冷却限制");
        assert!(
            !refresh_due(Some(now), now + Duration::from_secs(2)),
            "冷却期内不重复请求"
        );
        assert_eq!(REFRESH_COOLDOWN, Duration::from_secs(3));
        assert!(refresh_due(Some(now), now + REFRESH_COOLDOWN));
    }

    #[test]
    fn task_map_exposes_ui_fields() {
        let mut t = task("id1", TaskPhase::Running);
        t.name = "电影.mkv".to_owned();
        t.progress = 42;
        t.file_size = 123_456;
        t.kind = Some(EntryKind::File);
        t.file_id = Some("file-1".to_owned());
        t.thumbnail_url = "https://thumb.example/secret".to_owned();
        t.media = true;
        t.video = true;
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
        assert_eq!(get("kind"), "file");
        assert_eq!(get("fileId"), "file-1");
        assert_eq!(get("sizeText"), "121 KB");
        assert_eq!(
            map.get(&QString::from("hasThumbnail"))
                .and_then(|value| value.value::<bool>()),
            Some(true)
        );
        assert_eq!(
            map.get(&QString::from("video"))
                .and_then(|value| value.value::<bool>()),
            Some(true)
        );
        assert_eq!(
            map.get(&QString::from("playable"))
                .and_then(|value| value.value::<bool>()),
            Some(true)
        );
        assert!(map.get(&QString::from("thumbnailUrl")).is_none());
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
        let mut deleted = task("c", TaskPhase::Error);
        deleted.message = Some("File deleted".into());
        let deleted = task_map(&deleted);
        let read = |map: &QVariantMap| {
            map.get(&QString::from("settled"))
                .and_then(|v| v.value::<bool>())
        };
        assert_eq!(read(&running), Some(false));
        assert_eq!(read(&done), Some(true));
        assert_eq!(read(&deleted), Some(true));
    }

    #[test]
    fn deleted_output_is_not_presented_as_a_failure() {
        let mut deleted = task("deleted", TaskPhase::Error);
        deleted.message = Some("File deleted".into());
        let map = task_map(&deleted);
        let string = |key: &str| {
            map.get(&QString::from(key))
                .and_then(|value| value.value::<QString>())
                .map(|value| value.to_string())
        };
        assert_eq!(string("phase").as_deref(), Some("deleted"));
        assert_eq!(string("phaseText").as_deref(), Some("产物已删除"));
        assert_eq!(string("detail").as_deref(), Some("File deleted"));

        let mut failed = task("failed", TaskPhase::Error);
        failed.message = Some("Save failed, retry please".into());
        let map = task_map(&failed);
        assert_eq!(
            map.get(&QString::from("phase"))
                .and_then(|value| value.value::<QString>())
                .map(|value| value.to_string())
                .as_deref(),
            Some("error")
        );
    }
}
