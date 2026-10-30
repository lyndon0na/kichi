//! 全局限速：顺序下载与分片下载共用一个令牌桶。
//!
//! 令牌按当前 `session::limits` 的速率补充。等待切成 50ms，取消 / 暂停
//! 最多一个切片内生效；读到的字节会先写盘再因取消返回，不丢数据。

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::download::Cancel;
use super::limits;

const CANCEL_POLL: Duration = Duration::from_millis(50);
const MIN_CAPACITY: f64 = 64.0 * 1024.0;
const MAX_CAPACITY: f64 = 4.0 * 1024.0 * 1024.0;

struct Bucket {
    tokens: f64,
    at: Instant,
}

fn bucket() -> &'static Mutex<Bucket> {
    static BUCKET: OnceLock<Mutex<Bucket>> = OnceLock::new();
    BUCKET.get_or_init(|| {
        Mutex::new(Bucket {
            tokens: MIN_CAPACITY,
            at: Instant::now(),
        })
    })
}

/// 记入已经收到的字节；`false` = 等待期间被取消 / 暂停。
pub(crate) fn consume(cancel: &Cancel, bytes: usize) -> bool {
    if bytes == 0 || limits::rate_bytes_per_sec() == 0 {
        return true;
    }
    let bytes = bytes as f64;
    loop {
        if cancel.is_cancelled() {
            return false;
        }
        let rate = limits::rate_bytes_per_sec();
        if rate == 0 {
            return true;
        }
        let wait = {
            let mut bucket = bucket()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            refill(&mut bucket, Instant::now(), rate as f64);
            if bucket.tokens >= bytes {
                bucket.tokens -= bytes;
                return true;
            }
            duration_for((bytes - bucket.tokens) / rate as f64)
        };
        if !super::download::sleep_cancellable(cancel, wait.min(CANCEL_POLL)) {
            return false;
        }
    }
}

fn refill(bucket: &mut Bucket, now: Instant, rate: f64) {
    let elapsed = now.saturating_duration_since(bucket.at).as_secs_f64();
    let capacity = (rate / 4.0).clamp(MIN_CAPACITY, MAX_CAPACITY);
    bucket.tokens = (bucket.tokens + elapsed * rate).min(capacity);
    bucket.at = now;
}

fn duration_for(seconds: f64) -> Duration {
    let millis = (seconds.max(0.001) * 1000.0).ceil();
    Duration::from_millis(if millis.is_finite() && millis > 0.0 {
        millis.min(u64::MAX as f64) as u64
    } else {
        1
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_refills_to_a_bounded_capacity() {
        let now = Instant::now();
        let mut bucket = Bucket {
            tokens: 0.0,
            at: now,
        };
        refill(&mut bucket, now + Duration::from_secs(10), 1024.0 * 1024.0);
        assert_eq!(bucket.tokens, 256.0 * 1024.0);
        refill(&mut bucket, now + Duration::from_secs(30), 1024.0 * 1024.0);
        assert_eq!(bucket.tokens, 256.0 * 1024.0, "不会攒出无上限突发");
    }

    #[test]
    fn wait_duration_is_at_least_one_millisecond() {
        assert_eq!(duration_for(0.0), Duration::from_millis(1));
        assert_eq!(duration_for(0.001), Duration::from_millis(1));
        assert_eq!(duration_for(0.0015), Duration::from_millis(2));
    }
}
