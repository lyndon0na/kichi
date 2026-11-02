//! 上传全局限速：所有上传连接共享一个令牌桶。
//!
//! 读取文件 / 分片内存时按实际交给 reqwest 的字节扣令牌；取消时最多等
//! 一个 50ms 切片。速率由 GUI 设置层通过 [`UploadRateLimiter::set_rate`]
//! 在运行时调整，0 表示不限速。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

const CANCEL_POLL: Duration = Duration::from_millis(50);
const MIN_CAPACITY: f64 = 64.0 * 1024.0;
const MAX_CAPACITY: f64 = 4.0 * 1024.0 * 1024.0;

struct Bucket {
    tokens: f64,
    at: Instant,
}

pub struct UploadRateLimiter {
    rate_bytes_per_sec: AtomicU64,
    bucket: Mutex<Bucket>,
}

impl UploadRateLimiter {
    pub fn new(rate_bytes_per_sec: u64) -> Self {
        Self {
            rate_bytes_per_sec: AtomicU64::new(rate_bytes_per_sec),
            bucket: Mutex::new(Bucket {
                tokens: MIN_CAPACITY,
                at: Instant::now(),
            }),
        }
    }

    pub fn set_rate(&self, rate_bytes_per_sec: u64) {
        let previous = self
            .rate_bytes_per_sec
            .swap(rate_bytes_per_sec, Ordering::Relaxed);
        let now = Instant::now();
        let mut bucket = self
            .bucket
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if previous > 0 {
            refill(&mut bucket, now, previous as f64);
        }
        bucket.tokens = bucket.tokens.min(capacity_for(rate_bytes_per_sec as f64));
    }

    pub fn rate(&self) -> u64 {
        self.rate_bytes_per_sec.load(Ordering::Relaxed)
    }

    /// 记入已经读出的字节；`false` = 等待期间被取消。
    pub(crate) fn wait<C>(&self, bytes: usize, cancelled: &C) -> bool
    where
        C: Fn() -> bool,
    {
        if bytes == 0 || self.rate() == 0 {
            return true;
        }
        let bytes = bytes as f64;
        loop {
            if cancelled() {
                return false;
            }
            let rate = self.rate();
            if rate == 0 {
                return true;
            }
            let wait = {
                let mut bucket = self
                    .bucket
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                refill(&mut bucket, Instant::now(), rate as f64);
                if bucket.tokens >= bytes {
                    bucket.tokens -= bytes;
                    return true;
                }
                duration_for((bytes - bucket.tokens) / rate as f64)
            };
            thread::sleep(wait.min(CANCEL_POLL));
        }
    }
}

fn refill(bucket: &mut Bucket, now: Instant, rate: f64) {
    let elapsed = now.saturating_duration_since(bucket.at).as_secs_f64();
    let capacity = capacity_for(rate);
    bucket.tokens = (bucket.tokens + elapsed * rate).min(capacity);
    bucket.at = now;
}

fn capacity_for(rate: f64) -> f64 {
    (rate / 4.0).clamp(MIN_CAPACITY, MAX_CAPACITY)
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
    fn zero_rate_does_not_wait_and_cancelled_wait_stops() {
        let limiter = UploadRateLimiter::new(0);
        assert!(limiter.wait(1024 * 1024, &|| false));
        limiter.set_rate(1);
        assert!(!limiter.wait(1024 * 1024, &|| true));
    }

    #[test]
    fn wait_duration_is_at_least_one_millisecond() {
        assert_eq!(duration_for(0.0), Duration::from_millis(1));
        assert_eq!(duration_for(0.001), Duration::from_millis(1));
        assert_eq!(duration_for(0.0015), Duration::from_millis(2));
    }
}
