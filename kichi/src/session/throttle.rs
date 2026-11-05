//! 全局限速：顺序下载与分片下载共用一个令牌桶。
//!
//! 令牌按当前 `session::limits` 的速率补充。等待切成 50ms，取消 / 暂停
//! 最多一个切片内生效；读到的字节会先写盘再因取消返回，不丢数据。

use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use kichi_core::rate_bucket::{RateBucket, CANCEL_POLL};

use super::download::Cancel;
use super::limits;

fn bucket() -> &'static Mutex<RateBucket> {
    static BUCKET: OnceLock<Mutex<RateBucket>> = OnceLock::new();
    BUCKET.get_or_init(|| Mutex::new(RateBucket::new()))
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
            bucket.take(bytes, Instant::now(), rate as f64)
        };
        match wait {
            None => return true,
            Some(wait) => {
                if !super::download::sleep_cancellable(cancel, wait.min(CANCEL_POLL)) {
                    return false;
                }
            }
        }
    }
}
