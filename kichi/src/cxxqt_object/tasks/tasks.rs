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
use kichi_core::OfflineTask;

use super::actions::run_in_background;
use super::qobject;
use super::QVariantList;
use crate::session::{has_active, Failure};

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
        let (has_cache, restored) = {
            let mut rust = self.as_mut().rust_mut();
            let restored = if !rust.tasks_loaded && rust.task_list.is_empty() {
                restore_task_snapshot(&mut rust, &session)
            } else {
                false
            };
            (rust.tasks_loaded, restored)
        };
        if restored {
            // 快照换了 tasks 属性，任务页激活时先渲染出来，再静默校准。
            self.as_mut().notify();
        }
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
            rust.tasks_clearing = false;
            rust.tasks_request_running = false;
            rust.task_refresh_pending = false;
            rust.tasks_loaded = false;
            rust.task_refresh_at = None;
            rust.tasks = QVariantList::default();
            rust.task_list.clear();
            rust.notified.clear();
            rust.task_destinations.clear();
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
}

#[path = "tasks_actions.rs"]
mod tasks_actions;
#[path = "tasks_poll.rs"]
mod tasks_poll;
#[path = "tasks_view.rs"]
mod tasks_view;

#[cfg(test)]
pub(crate) use tasks_actions::finished_task_ids;
pub(crate) use tasks_poll::*;
pub(crate) use tasks_view::*;

#[cfg(test)]
#[path = "tasks_tests.rs"]
mod tests;
