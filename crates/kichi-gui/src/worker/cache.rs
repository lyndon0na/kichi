use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::msg::Msg;

use super::preview::preview_root;
use super::thumbs::thumbnail_cache_dir;

/// 磁盘缓存淘汰的节流: 距上次扫描超过该间隔就扫一次。
const CACHE_SWEEP_MIN_INTERVAL: Duration = Duration::from_secs(60);
/// 磁盘缓存淘汰的节流: 或累计新增超过该字节数时扫一次。
const CACHE_SWEEP_MIN_NEW_BYTES: u64 = 8 << 20;

/// 磁盘缓存的控制句柄: 「正在使用」登记 + 淘汰节流状态。
///
/// 从 `WorkerState` 里拆出来是为了能按值传进 spawn 出的任务。
#[derive(Clone)]
pub(super) struct CacheCtl {
    /// 正在写入 / 刚交给系统的缓存路径; 淘汰时跳过。
    in_use: Arc<Mutex<HashSet<PathBuf>>>,
    gate: Arc<Mutex<CacheGate>>,
}

/// 淘汰节流状态: 写入缓存后要累计到一定量或过一段时间, 才真去扫目录。
struct CacheGate {
    last: Instant,
    new_bytes: u64,
}

/// 缓存路径的「正在使用」登记; drop 时自动注销。
pub(super) struct InUseGuard {
    set: Arc<Mutex<HashSet<PathBuf>>>,
    paths: Vec<PathBuf>,
}

impl Drop for InUseGuard {
    fn drop(&mut self) {
        if let Ok(mut s) = self.set.lock() {
            for p in &self.paths {
                s.remove(p);
            }
        }
    }
}

impl CacheCtl {
    pub(super) fn new() -> Self {
        Self {
            in_use: Arc::new(Mutex::new(HashSet::new())),
            gate: Arc::new(Mutex::new(CacheGate {
                last: Instant::now(),
                new_bytes: 0,
            })),
        }
    }

    /// 登记一组正在使用的缓存路径(下载目标与 `.part`), 返回的守卫 drop 时注销。
    pub(super) fn mark_in_use(&self, paths: &[PathBuf]) -> InUseGuard {
        if let Ok(mut s) = self.in_use.lock() {
            s.extend(paths.iter().cloned());
        }
        InUseGuard {
            set: self.in_use.clone(),
            paths: paths.to_vec(),
        }
    }

    /// 记一笔缓存写入(字节数); 达到节流阈值就触发一次后台淘汰。
    pub(super) fn note_write(&self, bytes: u64) {
        let due = {
            let Ok(mut g) = self.gate.lock() else { return };
            g.new_bytes = g.new_bytes.saturating_add(bytes);
            if g.new_bytes < CACHE_SWEEP_MIN_NEW_BYTES
                && g.last.elapsed() < CACHE_SWEEP_MIN_INTERVAL
            {
                false
            } else {
                g.last = Instant::now();
                g.new_bytes = 0;
                true
            }
        };
        if due {
            tokio::spawn(sweep_caches(self.clone(), false, None));
        }
    }
}

/// 扫描两个缓存根: `purge=false` 按上限淘汰, `purge=true` 清空。
/// `reply` 非空时回传占用情况(设置页展示)。
pub(super) async fn sweep_caches(cache: CacheCtl, purge: bool, reply: Option<Sender<Msg>>) {
    // 先取一次「正在使用」快照, 供整个扫描过程使用。
    let snapshot: HashSet<PathBuf> = cache.in_use.lock().map(|g| g.clone()).unwrap_or_default();
    let roots: [(PathBuf, crate::cache::Caps); 2] = [
        (preview_root(), crate::cache::PREVIEW_CAPS),
        (thumbnail_cache_dir(), crate::cache::THUMB_CAPS),
    ];

    let result = tokio::task::spawn_blocking(move || {
        let mut total = crate::cache::Sweep::default();
        for (root, caps) in &roots {
            let s = if purge {
                crate::cache::purge(root, &snapshot)
            } else {
                crate::cache::evict_lru(root, caps, &snapshot)
            };
            total.bytes += s.bytes;
            total.entries += s.entries;
            total.removed += s.removed;
            total.freed += s.freed;
        }
        total
    })
    .await;

    match (reply, result) {
        (Some(tx), Ok(total)) => {
            if total.removed > 0 {
                tracing::info!(
                    "缓存清理: 删除 {} 项, 释放 {} 字节",
                    total.removed,
                    total.freed
                );
            }
            let _ = tx.send(Msg::CacheUsage {
                bytes: total.bytes,
                entries: total.entries,
                freed: total.freed,
            });
        }
        (_, Err(e)) => tracing::warn!("缓存扫描任务失败: {e}"),
        _ => {}
    }
}
