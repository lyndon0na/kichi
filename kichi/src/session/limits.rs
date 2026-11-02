//! 下载 / 上传运行参数：并发、分片数与全局限速。
//!
//! 设置页写入运行时值；`KICHI_DOWNLOAD_*` 环境变量继续拥有更高优先级，
//! 方便出问题时不改设置文件直接回滚。非法 / 空环境变量视为未设置。

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

use kichi_core::UploadRateLimiter;

pub(crate) const PARALLEL_ENV: &str = "KICHI_DOWNLOAD_PARALLEL";
pub(crate) const CONNECTIONS_ENV: &str = "KICHI_DOWNLOAD_CONNECTIONS";
pub(crate) const RATE_ENV: &str = "KICHI_DOWNLOAD_RATE_LIMIT_MIB";

pub(crate) const DEFAULT_PARALLEL: usize = 3;
pub(crate) const DEFAULT_CONNECTIONS: usize = 4;
pub(crate) const MAX_PARALLEL: usize = 8;
pub(crate) const MAX_CONNECTIONS: usize = 8;
/// 设置页与文档统一使用 0–1024 MiB/s；0 单独表示不限速。
pub(crate) const MAX_RATE_MIB: u64 = 1024;
const MIB: u64 = 1024 * 1024;

pub(crate) const UPLOAD_PARALLEL_ENV: &str = "KICHI_UPLOAD_PARALLEL";
pub(crate) const UPLOAD_RATE_ENV: &str = "KICHI_UPLOAD_RATE_LIMIT_MIB";

pub(crate) const DEFAULT_UPLOAD_PARALLEL: usize = 2;
pub(crate) const MAX_UPLOAD_PARALLEL: usize = 8;
/// 上传限速与下载共用 0–1024 MiB/s 的界面范围；0 = 不限。
pub(crate) const MAX_UPLOAD_RATE_MIB: u64 = MAX_RATE_MIB;

fn parallel_value() -> &'static AtomicUsize {
    static VALUE: AtomicUsize = AtomicUsize::new(DEFAULT_PARALLEL);
    &VALUE
}

fn connections_value() -> &'static AtomicUsize {
    static VALUE: AtomicUsize = AtomicUsize::new(DEFAULT_CONNECTIONS);
    &VALUE
}

fn rate_mib_value() -> &'static AtomicU64 {
    static VALUE: AtomicU64 = AtomicU64::new(0);
    &VALUE
}

fn upload_parallel_value() -> &'static AtomicUsize {
    static VALUE: AtomicUsize = AtomicUsize::new(DEFAULT_UPLOAD_PARALLEL);
    &VALUE
}

fn upload_rate_mib_value() -> &'static AtomicU64 {
    static VALUE: AtomicU64 = AtomicU64::new(0);
    &VALUE
}

/// 设置页改完设置后写入；越界值在入口就收回。
pub(crate) fn configure(parallel: usize, connections: usize, rate_mib: u64) {
    parallel_value().store(parallel.clamp(1, MAX_PARALLEL), Ordering::Relaxed);
    connections_value().store(connections.clamp(1, MAX_CONNECTIONS), Ordering::Relaxed);
    rate_mib_value().store(rate_mib.min(MAX_RATE_MIB), Ordering::Relaxed);
}

/// 上传运行参数；下载与上传各自独立，互不换算。
pub(crate) fn configure_uploads(parallel: usize, rate_mib: u64) {
    upload_parallel_value().store(parallel.clamp(1, MAX_UPLOAD_PARALLEL), Ordering::Relaxed);
    upload_rate_mib_value().store(rate_mib.min(MAX_UPLOAD_RATE_MIB), Ordering::Relaxed);
    upload_limiter().set_rate(upload_rate_bytes_per_sec());
}

/// 同时下载的文件数（跨批与文件夹批内共用）。
pub(crate) fn parallel() -> usize {
    env_usize(PARALLEL_ENV, MAX_PARALLEL)
        .unwrap_or_else(|| parallel_value().load(Ordering::Relaxed))
        .clamp(1, MAX_PARALLEL)
}

/// 单文件分片连接数；1 = 不分片。
pub(crate) fn connections() -> usize {
    env_usize(CONNECTIONS_ENV, MAX_CONNECTIONS)
        .unwrap_or_else(|| connections_value().load(Ordering::Relaxed))
        .clamp(1, MAX_CONNECTIONS)
}

/// 全局限速（MiB/s）；0 = 不限。
pub(crate) fn rate_mib() -> u64 {
    env_rate_mib().unwrap_or_else(|| rate_mib_value().load(Ordering::Relaxed))
}

/// 全局限速（bytes/s）；0 = 不限。
pub(crate) fn rate_bytes_per_sec() -> u64 {
    rate_mib().saturating_mul(MIB)
}

/// 同时上传的文件数。单文件批次跨批共用；文件夹批次独占启动后批内共用。
pub(crate) fn upload_parallel() -> usize {
    env_usize(UPLOAD_PARALLEL_ENV, MAX_UPLOAD_PARALLEL)
        .unwrap_or_else(|| upload_parallel_value().load(Ordering::Relaxed))
        .clamp(1, MAX_UPLOAD_PARALLEL)
}

/// 全局限速（MiB/s）；0 = 不限。环境变量优先级高于设置页。
pub(crate) fn upload_rate_mib() -> u64 {
    env_upload_rate_mib().unwrap_or_else(|| upload_rate_mib_value().load(Ordering::Relaxed))
}

/// 全局限速（bytes/s）；0 = 不限。
pub(crate) fn upload_rate_bytes_per_sec() -> u64 {
    upload_rate_mib().saturating_mul(MIB)
}

/// 所有上传请求共享的令牌桶；设置变化时由 [`configure_uploads`] 原地改速率。
pub(crate) fn upload_limiter() -> Arc<UploadRateLimiter> {
    static LIMITER: OnceLock<Arc<UploadRateLimiter>> = OnceLock::new();
    Arc::clone(
        LIMITER.get_or_init(|| Arc::new(UploadRateLimiter::new(upload_rate_bytes_per_sec()))),
    )
}

fn env_usize(name: &str, max: usize) -> Option<usize> {
    let raw = std::env::var(name).ok()?;
    raw.trim()
        .parse::<usize>()
        .ok()
        .filter(|value| *value > 0)
        .map(|value| value.min(max))
}

fn env_rate_mib() -> Option<u64> {
    let raw = std::env::var(RATE_ENV).ok()?;
    raw.trim()
        .parse::<u64>()
        .ok()
        .map(|value| value.min(MAX_RATE_MIB))
}

fn env_upload_rate_mib() -> Option<u64> {
    let raw = std::env::var(UPLOAD_RATE_ENV).ok()?;
    raw.trim()
        .parse::<u64>()
        .ok()
        .map(|value| value.min(MAX_UPLOAD_RATE_MIB))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_values_are_clamped() {
        configure(0, 99, MAX_RATE_MIB + 1);
        assert_eq!(parallel_value().load(Ordering::Relaxed), 1);
        assert_eq!(connections_value().load(Ordering::Relaxed), MAX_CONNECTIONS);
        assert_eq!(rate_mib_value().load(Ordering::Relaxed), MAX_RATE_MIB);
        configure(DEFAULT_PARALLEL, DEFAULT_CONNECTIONS, 0);

        configure_uploads(0, MAX_UPLOAD_RATE_MIB + 1);
        assert_eq!(upload_parallel_value().load(Ordering::Relaxed), 1);
        assert_eq!(
            upload_rate_mib_value().load(Ordering::Relaxed),
            MAX_UPLOAD_RATE_MIB
        );
        assert_eq!(upload_limiter().rate(), MAX_UPLOAD_RATE_MIB * MIB);
        configure_uploads(DEFAULT_UPLOAD_PARALLEL, 0);
    }

    #[test]
    fn environment_parsers_reject_empty_zero_and_garbage() {
        assert_eq!(env_usize("KICHI_TEST_LIMIT_UNSET", 8), None);
        for raw in ["", "0", "-1", "abc"] {
            std::env::set_var("KICHI_TEST_LIMIT_PARSE", raw);
            assert_eq!(env_usize("KICHI_TEST_LIMIT_PARSE", 8), None, "{raw:?}");
        }
        std::env::set_var("KICHI_TEST_LIMIT_PARSE", "99");
        assert_eq!(env_usize("KICHI_TEST_LIMIT_PARSE", 8), Some(8));
        std::env::remove_var("KICHI_TEST_LIMIT_PARSE");

        std::env::set_var(RATE_ENV, "0");
        assert_eq!(env_rate_mib(), Some(0));
        std::env::set_var(RATE_ENV, "12");
        assert_eq!(env_rate_mib(), Some(12));
        std::env::set_var(RATE_ENV, "abc");
        assert_eq!(env_rate_mib(), None);
        std::env::remove_var(RATE_ENV);
    }
}
