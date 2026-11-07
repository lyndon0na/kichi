//! 后台轮询线程与冷启动快照恢复。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

use cxx_qt_lib::QVariant;
use kichi_core::OfflineTask;

use crate::cxxqt_object::actions::enqueue_ui;
use crate::cxxqt_object::KichiRust;
use crate::cxxqt_object::QVariantList;
use crate::session::{page_snapshot, Session};

use super::*;

/// 冷启动恢复上次成功列表；返回是否恢复出了行。
///
/// 不恢复 `known_running`：任务若在应用关闭期间完成，首次校准不补发系统通知。
pub(crate) fn restore_task_snapshot(rust: &mut KichiRust, session: &Session) -> bool {
    let Some(tasks) =
        page_snapshot::load::<Vec<OfflineTask>>(session, page_snapshot::OFFLINE_TASKS)
    else {
        return false;
    };
    let mut list = QVariantList::default();
    for task in &tasks {
        list.append(QVariant::from(&task_map(task)));
    }
    rust.tasks = list;
    rust.task_list = tasks;
    rust.tasks_loaded = true;
    // 快照只负责占位：保持刷新到期时间为空，首轮校准静默进行。
    rust.task_refresh_at = None;
    true
}

/// 后台轮询循环：按间隔拉列表，结果回投到主线程。
///
/// 所有请求都回到主线程调用后台刷新入口，与切页 / 手动刷新共用内部单飞保护。
/// 每 TICK 醒来一次检查退出或间隔变化。
pub(crate) fn poll_loop(generation: u64) {
    loop {
        let interval = poll_wait().load(Ordering::SeqCst);
        let revision = poll_wait_revision().load(Ordering::SeqCst);
        match wait_for_poll(Duration::from_secs(interval), generation, revision) {
            PollWait::Stopped => return,
            PollWait::Rescheduled => continue,
            PollWait::Due => {}
        }
        enqueue_ui(move |mut qichi| {
            if still_polling(generation) {
                qichi.as_mut().refresh_tasks_background();
            }
        });
    }
}

/// 下一轮轮询的间隔（秒）；成功刷新后会按任务状态重新设置。
pub(crate) fn poll_wait() -> &'static AtomicU64 {
    static WAIT: OnceLock<AtomicU64> = OnceLock::new();
    WAIT.get_or_init(|| AtomicU64::new(POLL_ACTIVE.as_secs()))
}

/// 轮询间隔版本：成功刷新后重置倒计时，手动刷新发现活动任务时也能尽快切回 10 秒。
pub(crate) fn poll_wait_revision() -> &'static AtomicU64 {
    static REVISION: OnceLock<AtomicU64> = OnceLock::new();
    REVISION.get_or_init(|| AtomicU64::new(0))
}

pub(crate) fn schedule_poll_wait(interval: Duration) {
    poll_wait().store(interval.as_secs(), Ordering::SeqCst);
    poll_wait_revision().fetch_add(1, Ordering::SeqCst);
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum PollWait {
    Due,
    Rescheduled,
    Stopped,
}

/// 等到下一轮该拉取，或因为间隔变化 / 登出提前结束。
pub(crate) fn wait_for_poll(total: Duration, generation: u64, revision: u64) -> PollWait {
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
pub(crate) fn still_polling(generation: u64) -> bool {
    poll_running().load(Ordering::SeqCst) && poll_generation().load(Ordering::SeqCst) == generation
}
