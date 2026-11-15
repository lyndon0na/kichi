//! 登录保活节拍。
//!
//! `pump()` 每 100ms 跑一次，这里把它压成 60 秒一次：到点才允许后台线程
//! 去检查令牌是否临期并刷新。`busy` 保证同一时刻只有一次刷新 / 重登在途。

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// 检查间隔。令牌提前 5 分钟刷新，因此最多晚 1 分钟进入刷新窗口。
pub(crate) const CHECK_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Debug)]
struct State {
    next: Instant,
    busy: bool,
}

impl State {
    fn new(now: Instant) -> Self {
        Self {
            // 启动恢复已经自己刷过一次，首轮检查留出一个完整周期
            next: now + CHECK_INTERVAL,
            busy: false,
        }
    }

    fn begin(&mut self, now: Instant, force: bool) -> bool {
        if self.busy || (!force && now < self.next) {
            return false;
        }
        self.busy = true;
        self.next = now + CHECK_INTERVAL;
        true
    }

    fn finish(&mut self) {
        self.busy = false;
    }
}

fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(State::new(Instant::now())))
}

/// 本次 `pump` 是否该启动一次保活；返回 `true` 的调用方必须最终 [`finish`]。
pub(crate) fn begin() -> bool {
    begin_in(false)
}

/// 窗口重新获得焦点时强制查一次（例如合盖休眠过），仍遵守 `busy` 单飞。
pub(crate) fn begin_now() -> bool {
    begin_in(true)
}

fn begin_in(force: bool) -> bool {
    state()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .begin(Instant::now(), force)
}

/// 一轮保活结束（成功、失败或线程没起来都要调）。
pub(crate) fn finish() {
    state()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .finish();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_check_waits_one_interval() {
        let start = Instant::now();
        let mut state = State::new(start);
        assert!(!state.begin(start, false));
        assert!(!state.begin(start + CHECK_INTERVAL - Duration::from_secs(1), false));
        assert!(state.begin(start + CHECK_INTERVAL, false));
    }

    #[test]
    fn focus_can_check_before_the_next_tick() {
        let start = Instant::now();
        let mut state = State::new(start);
        assert!(state.begin(start, true));
        assert!(!state.begin(start, true), "在途时焦点不能并发再起一轮");
        state.finish();
        assert!(state.begin(start, true));
    }

    #[test]
    fn busy_blocks_overlapping_runs() {
        let start = Instant::now();
        let mut state = State::new(start);
        assert!(state.begin(start + CHECK_INTERVAL, false));
        assert!(!state.begin(start + CHECK_INTERVAL * 2, false));
        state.finish();
        assert!(state.begin(start + CHECK_INTERVAL * 2, false));
    }
}
