//! 上传全局限速：所有上传连接共享一个令牌桶。
//!
//! 读取文件 / 分片内存时按实际交给 reqwest 的字节扣令牌；取消时最多等
//! 一个 50ms 切片。速率由 GUI 设置层通过 [`UploadRateLimiter::set_rate`]
//! 在运行时调整，0 表示不限速。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::Instant;

use crate::rate_bucket::{RateBucket, CANCEL_POLL};

pub struct UploadRateLimiter {
    rate_bytes_per_sec: AtomicU64,
    bucket: Mutex<RateBucket>,
}

impl UploadRateLimiter {
    pub fn new(rate_bytes_per_sec: u64) -> Self {
        Self {
            rate_bytes_per_sec: AtomicU64::new(rate_bytes_per_sec),
            bucket: Mutex::new(RateBucket::new()),
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
        // 先按旧速率结算已积累的令牌，再把突发压回新速率的容量
        if previous > 0 {
            bucket.refill(now, previous as f64);
        }
        bucket.clamp_to_capacity(rate_bytes_per_sec as f64);
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
                bucket.take(bytes, Instant::now(), rate as f64)
            };
            match wait {
                None => return true,
                Some(wait) => thread::sleep(wait.min(CANCEL_POLL)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_rate_does_not_wait_and_cancelled_wait_stops() {
        let limiter = UploadRateLimiter::new(0);
        assert!(limiter.wait(1024 * 1024, &|| false));
        limiter.set_rate(1);
        assert!(!limiter.wait(1024 * 1024, &|| true));
    }
}
