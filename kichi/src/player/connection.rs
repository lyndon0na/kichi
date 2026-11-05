//! mpv 与 VLC 共用的 socket 连接重试。
//!
//! 播放器刚 spawn 时 socket 往往还没建好，两个后端都是「5 秒内每 20ms 试一次，
//! 期间可取消」。单次尝试（读超时设置、连接结构）各后端不同，这里只收口重试循环。

use std::io;
use std::time::{Duration, Instant};

use crate::session::download::Cancel;

/// 连 socket 的总宽限（播放器实测 0.13–0.15 秒就绪）。
pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// 每次重连的间隔。
pub(crate) const CONNECT_RETRY: Duration = Duration::from_millis(20);

/// 反复调用 `try_connect` 直到成功、取消或超时；失败细节由单次尝试自行丢弃。
pub(crate) fn connect_with<T>(
    cancel: &Cancel,
    timeout: Duration,
    retry: Duration,
    mut try_connect: impl FnMut() -> Option<T>,
) -> Option<T> {
    let deadline = Instant::now() + timeout;
    loop {
        if cancel.is_cancelled() {
            return None;
        }
        if let Some(connection) = try_connect() {
            return Some(connection);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(retry);
    }
}

/// 读超时（`WouldBlock` / `TimedOut`）不算连接断开。
pub(crate) fn is_timeout(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_until_the_first_success() {
        let cancel = Cancel::default();
        let mut attempts = 0;
        let result = connect_with(
            &cancel,
            Duration::from_millis(100),
            Duration::from_millis(1),
            || {
                attempts += 1;
                (attempts >= 3).then_some("connected")
            },
        );
        assert_eq!(result, Some("connected"));
        assert_eq!(attempts, 3);
    }

    #[test]
    fn gives_up_after_the_timeout() {
        let cancel = Cancel::default();
        let started = Instant::now();
        let mut attempts = 0;
        let result = connect_with(
            &cancel,
            Duration::from_millis(10),
            Duration::from_millis(5),
            || {
                attempts += 1;
                None::<()>
            },
        );
        assert!(result.is_none());
        assert!(
            started.elapsed() >= Duration::from_millis(10),
            "至少等到总超时"
        );
        assert!(attempts >= 2);
    }

    #[test]
    fn a_cancelled_signal_skips_every_attempt() {
        let cancel = Cancel::default();
        cancel.cancel();
        let mut attempts = 0;
        let result = connect_with(
            &cancel,
            Duration::from_secs(5),
            Duration::from_millis(20),
            || {
                attempts += 1;
                Some(1)
            },
        );
        assert!(result.is_none());
        assert_eq!(attempts, 0);
    }

    #[test]
    fn only_io_timeouts_are_timeouts() {
        assert!(is_timeout(&io::Error::from(io::ErrorKind::WouldBlock)));
        assert!(is_timeout(&io::Error::from(io::ErrorKind::TimedOut)));
        assert!(!is_timeout(&io::Error::from(
            io::ErrorKind::ConnectionRefused
        )));
    }
}
