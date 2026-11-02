//! 上传实时进度的共享计数器。
//!
//! 上传体由 reqwest 内部线程读取，`Body::sized` 还要求读取器 `'static`，
//! 所以读器不能借用 `FnMut` 进度回调；它只累加原子计数，由上传主线程按
//! [`LIVE_INTERVAL`] 汇总成 `UploadProgress`。
//!
//! **在途字节只用于显示**；断点与传输历史只认已完成分片字节。

use std::collections::BTreeMap;
use std::io::{self, Read};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::upload::{UploadProgress, UploadStage};

/// 实时进度的重算间隔；应用层还有一层 250ms 节流。
pub(super) const LIVE_INTERVAL: Duration = Duration::from_millis(200);

/// 已完成分片 + 各在途分片已读出的字节。
pub(super) struct UploadByteTracker {
    completed: AtomicU64,
    active: Mutex<BTreeMap<u64, Arc<AtomicU64>>>,
}

impl UploadByteTracker {
    pub(super) fn new(completed: u64) -> Self {
        Self {
            completed: AtomicU64::new(completed),
            active: Mutex::new(BTreeMap::new()),
        }
    }

    pub(super) fn completed(&self) -> u64 {
        self.completed.load(Ordering::Relaxed)
    }

    pub(super) fn set_completed(&self, bytes: u64) {
        self.completed.store(bytes, Ordering::Relaxed);
    }

    /// 开始（或重试）一个分片：同名计数器归零，旧尝试的残量不再计入。
    pub(super) fn register(&self, key: u64) -> Arc<AtomicU64> {
        let counter = Arc::new(AtomicU64::new(0));
        self.active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(key, Arc::clone(&counter));
        counter
    }

    pub(super) fn unregister(&self, key: u64) {
        self.active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&key);
    }

    /// 当前显示用的已上传字节；夹到 `total`，避免分片完成消息尚未处理时超过 100%。
    pub(super) fn live_bytes(&self, total: u64) -> u64 {
        let mut bytes = self.completed();
        for counter in self
            .active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .values()
        {
            bytes = bytes.saturating_add(counter.load(Ordering::Relaxed));
        }
        bytes.min(total)
    }

    pub(super) fn progress(&self, total: u64) -> UploadProgress {
        UploadProgress {
            stage: UploadStage::Uploading,
            bytes: self.live_bytes(total),
            completed_bytes: Some(self.completed()),
            total,
        }
    }
}

/// 读取请求体时累计已交给 HTTP 的字节。
///
/// 必须包在限速器外层：限速等待完成后才记账，界面不会在令牌桶等待期间
/// 提前冲出一段进度。
pub(super) struct ProgressReader<R> {
    inner: R,
    counter: Arc<AtomicU64>,
}

impl<R> ProgressReader<R> {
    pub(super) fn new(inner: R, counter: Arc<AtomicU64>) -> Self {
        Self { inner, counter }
    }
}

impl<R: Read> Read for ProgressReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buf)?;
        if read > 0 {
            self.counter.fetch_add(read as u64, Ordering::Relaxed);
        }
        Ok(read)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn tracker_sums_completed_and_active_bytes_with_a_cap() {
        let tracker = UploadByteTracker::new(5);
        let first = tracker.register(1);
        let second = tracker.register(2);
        first.fetch_add(3, Ordering::Relaxed);
        second.fetch_add(4, Ordering::Relaxed);

        assert_eq!(tracker.completed(), 5);
        assert_eq!(tracker.live_bytes(10), 10, "在途字节不能把进度推过总量");
        assert_eq!(tracker.live_bytes(20), 12);
    }

    #[test]
    fn registering_the_same_part_resets_it_for_a_retry() {
        let tracker = UploadByteTracker::new(5);
        let first = tracker.register(7);
        first.fetch_add(4, Ordering::Relaxed);
        assert_eq!(tracker.live_bytes(100), 9);

        tracker.register(7);
        assert_eq!(tracker.live_bytes(100), 5, "重试的旧字节必须回退");
    }

    #[test]
    fn unregistering_a_completed_part_does_not_double_count() {
        let tracker = UploadByteTracker::new(5);
        let part = tracker.register(3);
        part.fetch_add(10, Ordering::Relaxed);
        tracker.unregister(3);
        tracker.set_completed(15);
        assert_eq!(tracker.live_bytes(20), 15);
    }

    #[test]
    fn progress_reader_counts_reads_and_ignores_eof() {
        let counter = Arc::new(AtomicU64::new(0));
        let mut reader =
            ProgressReader::new(Cursor::new(b"payload".to_vec()), Arc::clone(&counter));
        let mut first = [0u8; 3];
        assert_eq!(reader.read(&mut first).expect("read"), 3);
        let mut rest = Vec::new();
        reader.read_to_end(&mut rest).expect("read rest");
        assert_eq!(rest, b"load");
        assert_eq!(counter.load(Ordering::Relaxed), 7);

        let mut eof = [0u8; 1];
        assert_eq!(reader.read(&mut eof).expect("eof"), 0);
        assert_eq!(counter.load(Ordering::Relaxed), 7);
    }
}
