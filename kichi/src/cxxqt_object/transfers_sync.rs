//! 传输列表结构同步的节流器。
//!
//! 大文件夹上传时，每个文件开始 / 完成、每个云端目录创建都会产生结构变化。
//! 每条都重建一次 QML 模型既浪费主线程时间，也会让列表频繁重排；这里把
//! 这类后台通知合并成一个至少间隔 [`SYNC_INTERVAL`] 的尾随刷新。
//!
//! 事件只置脏，不丢；`Kichi::pump()` 每 100ms 检查一次是否到期，因此用户
//! 实际看到的是约 100–200ms 一次的合并刷新。结束 / 失败等最终状态仍走
//! 直接 `sync_transfers()`，不会等节流窗口。

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

pub(crate) const SYNC_INTERVAL: Duration = Duration::from_millis(150);

#[derive(Debug)]
struct Coalescer {
    interval: Duration,
    last_sync: Option<Instant>,
    dirty_since: Option<Instant>,
    requested: u64,
    synced: u64,
}

impl Default for Coalescer {
    fn default() -> Self {
        Self::new(SYNC_INTERVAL)
    }
}

impl Coalescer {
    fn new(interval: Duration) -> Self {
        Self {
            interval,
            last_sync: None,
            dirty_since: None,
            requested: 0,
            synced: 0,
        }
    }

    #[cfg(test)]
    fn at(interval: Duration, last_sync: Instant) -> Self {
        Self {
            last_sync: Some(last_sync),
            ..Self::new(interval)
        }
    }

    fn request(&mut self, now: Instant) {
        self.requested = self.requested.saturating_add(1);
        if self.dirty_since.is_none() {
            self.dirty_since = Some(now);
        }
    }

    fn due(&self, now: Instant) -> bool {
        self.dirty_since.is_some()
            && self
                .last_sync
                .is_none_or(|last| now.saturating_duration_since(last) >= self.interval)
    }

    fn generation(&self) -> u64 {
        self.requested
    }

    /// 完成一轮同步。只清掉快照开始前已经发生的事件；同步期间新到的事件
    /// 留到下一轮，避免被这一轮没读到的快照吞掉。
    fn finish(&mut self, covered: u64, now: Instant) {
        self.synced = self.synced.max(covered);
        self.last_sync = Some(now);
        if self.requested <= self.synced {
            self.dirty_since = None;
        }
    }
}

fn coalescer() -> &'static Mutex<Coalescer> {
    static STATE: OnceLock<Mutex<Coalescer>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(Coalescer::default()))
}

/// 后台线程产生了一次结构变化：等主线程下一轮到期时合并刷新。
pub(crate) fn request() {
    coalescer()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .request(Instant::now());
}

/// 主线程是否该补一次结构同步。
pub(crate) fn due() -> bool {
    coalescer()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .due(Instant::now())
}

/// `sync_transfers()` 开始取快照前的代数。
pub(crate) fn generation() -> u64 {
    coalescer()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .generation()
}

/// `sync_transfers()` 把快照刷到属性之后调用。
pub(crate) fn finish(covered: u64) {
    coalescer()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .finish(covered, Instant::now());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_within_the_window_are_coalesced() {
        let base = Instant::now();
        let mut state = Coalescer::at(Duration::from_millis(150), base);

        state.request(base);
        state.request(base + Duration::from_millis(80));
        assert!(!state.due(base + Duration::from_millis(99)));
        assert!(!state.due(base + Duration::from_millis(149)));
        assert!(state.due(base + Duration::from_millis(150)));

        let covered = state.generation();
        state.finish(covered, base + Duration::from_millis(150));
        assert!(!state.due(base + Duration::from_millis(199)));
        assert!(!state.due(base + Duration::from_millis(299)));
    }

    #[test]
    fn trailing_request_is_not_lost_by_an_in_flight_snapshot() {
        let base = Instant::now();
        let mut state = Coalescer::at(Duration::from_millis(150), base);
        state.request(base + Duration::from_millis(10));
        let covered = state.generation();

        // 快照已经开始取数据，后台又推来一条结构变化。
        state.request(base + Duration::from_millis(20));
        state.finish(covered, base + Duration::from_millis(150));

        assert!(!state.due(base + Duration::from_millis(299)));
        assert!(state.due(base + Duration::from_millis(300)));
    }

    #[test]
    fn a_sync_without_pending_events_marks_the_window() {
        let base = Instant::now();
        let mut state = Coalescer::at(Duration::from_millis(150), base);
        state.finish(state.generation(), base + Duration::from_millis(100));
        assert!(!state.due(base + Duration::from_millis(249)));
        state.request(base + Duration::from_millis(120));
        assert!(!state.due(base + Duration::from_millis(249)));
        assert!(state.due(base + Duration::from_millis(250)));
    }
}
