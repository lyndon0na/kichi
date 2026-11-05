//! 通用令牌桶：下载与上传的全局限速共用这一份纯逻辑。
//!
//! 桶按字节速率补充令牌，容量是速率的四分之一，并夹在 64 KiB–4 MiB 之间：
//! 下限保证低速时也能一次拿走一个网络块，上限避免长时间空闲后攒出巨大突发。
//! 等令牌的调用方把等待切成 [`CANCEL_POLL`]，取消 / 暂停最坏多等一个切片。
//!
//! 这里只管「够不够、还差多久」，不负责睡眠、取消标志或速率来源：
//! 那些由调用方（上传限速器、下载限速器）各自提供。时钟由调用方传入，
//! 便于单元测试。

use std::time::{Duration, Instant};

/// 等令牌的切片长度；调用方每次最多睡这么久，保证取消能及时生效。
pub const CANCEL_POLL: Duration = Duration::from_millis(50);

const MIN_CAPACITY: f64 = 64.0 * 1024.0;
const MAX_CAPACITY: f64 = 4.0 * 1024.0 * 1024.0;

/// 一个按字节速率补充的令牌桶。
#[derive(Debug)]
pub struct RateBucket {
    tokens: f64,
    at: Instant,
}

impl Default for RateBucket {
    fn default() -> Self {
        Self::new()
    }
}

impl RateBucket {
    /// 新桶带 64 KiB 启动令牌，允许连接刚开始时立刻读一个网络块。
    pub fn new() -> Self {
        Self {
            tokens: MIN_CAPACITY,
            at: Instant::now(),
        }
    }

    /// 按 `rate_bytes_per_sec` 把 `now` 之前流逝的时间补成令牌，并封顶到容量。
    pub fn refill(&mut self, now: Instant, rate_bytes_per_sec: f64) {
        let elapsed = now.saturating_duration_since(self.at).as_secs_f64();
        let capacity = Self::capacity_for(rate_bytes_per_sec);
        self.tokens = (self.tokens + elapsed * rate_bytes_per_sec).min(capacity);
        self.at = now;
    }

    /// 尝试记入 `bytes`：够就扣掉返回 `None`，不够返回还需等待的时间（不扣）。
    ///
    /// 调用方保证 `rate_bytes_per_sec > 0`（不限速时不会走到这里）。
    pub fn take(&mut self, bytes: f64, now: Instant, rate_bytes_per_sec: f64) -> Option<Duration> {
        self.refill(now, rate_bytes_per_sec);
        if self.tokens >= bytes {
            self.tokens -= bytes;
            None
        } else {
            Some(duration_for((bytes - self.tokens) / rate_bytes_per_sec))
        }
    }

    /// 速率变化后把已有令牌压回新速率允许的容量。
    pub fn clamp_to_capacity(&mut self, rate_bytes_per_sec: f64) {
        self.tokens = self.tokens.min(Self::capacity_for(rate_bytes_per_sec));
    }

    /// 速率对应的桶容量：速率的 1/4，夹在 64 KiB–4 MiB。
    pub fn capacity_for(rate_bytes_per_sec: f64) -> f64 {
        (rate_bytes_per_sec / 4.0).clamp(MIN_CAPACITY, MAX_CAPACITY)
    }

    /// 当前令牌数（测试与调试用）。
    pub fn tokens(&self) -> f64 {
        self.tokens
    }
}

/// 秒数换等待时长：至少 1 ms，非有限值也回 1 ms。
pub fn duration_for(seconds: f64) -> Duration {
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
        let mut bucket = RateBucket {
            tokens: 0.0,
            at: now,
        };
        bucket.refill(now + Duration::from_secs(10), 1024.0 * 1024.0);
        assert_eq!(bucket.tokens(), 256.0 * 1024.0);
        bucket.refill(now + Duration::from_secs(30), 1024.0 * 1024.0);
        assert_eq!(bucket.tokens(), 256.0 * 1024.0, "不会攒出无上限突发");
    }

    #[test]
    fn new_bucket_starts_with_one_network_block() {
        assert_eq!(RateBucket::new().tokens(), 64.0 * 1024.0);
    }

    #[test]
    fn take_deducts_or_reports_the_shortfall() {
        let now = Instant::now();
        let mut bucket = RateBucket {
            tokens: 0.0,
            at: now,
        };
        assert_eq!(
            bucket.take(100.0, now, 1000.0),
            Some(Duration::from_millis(100)),
            "不够时只报还差多久，不扣令牌"
        );
        assert_eq!(bucket.tokens(), 0.0);
        assert_eq!(
            bucket.take(100.0, now + Duration::from_millis(100), 1000.0),
            None
        );
        assert_eq!(bucket.tokens(), 0.0);
    }

    #[test]
    fn clamp_to_capacity_curbs_the_old_burst() {
        let now = Instant::now();
        let mut bucket = RateBucket {
            tokens: 0.0,
            at: now,
        };
        bucket.refill(now + Duration::from_secs(10), 1024.0 * 1024.0);
        bucket.clamp_to_capacity(64.0 * 1024.0);
        assert_eq!(bucket.tokens(), 64.0 * 1024.0);
    }

    #[test]
    fn wait_duration_is_at_least_one_millisecond() {
        assert_eq!(duration_for(0.0), Duration::from_millis(1));
        assert_eq!(duration_for(0.001), Duration::from_millis(1));
        assert_eq!(duration_for(0.0015), Duration::from_millis(2));
    }
}
