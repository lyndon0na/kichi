//! 片尾预取：临近结束时取下一集一条直链，存内存单槽。
//!
//! 从 `playlist.rs` 拆出来守行数。队列本身只存元数据；这里负责唯一一条
//! 预取直链的取、存、校验与丢弃，永远不落盘、不进日志。

use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use cxx_qt::CxxQtType;
use kichi_core::{PlayOption, PlaybackProgress};

use super::actions::run_in_background;
use super::qobject;
use crate::session::download::Cancel;
use crate::session::media;

/// 片尾前多少秒开始预取下一集；`KICHI_PLAY_PREFETCH_SECS=0` 关闭。
const DEFAULT_PREFETCH_SECS: u64 = 120;
/// 预取结果最多保留多久：暂停太久就重新取，避免拿着快过期的链接开播。
pub(super) const PREFETCH_MAX_AGE: Duration = Duration::from_secs(15 * 60);

fn prefetch_seconds() -> u64 {
    std::env::var("KICHI_PLAY_PREFETCH_SECS")
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(DEFAULT_PREFETCH_SECS)
}

/// 一条预取的下一集直链（只在内存里，不落盘、不进日志）。
pub(crate) struct PrefetchedLink {
    pub(super) id: String,
    pub(super) options: Vec<PlayOption>,
    pub(super) fetched_at: Instant,
}

impl qobject::Kichi {
    /// 片尾临近时后台预取下一集一条直链；同一时间最多一条在途。
    pub(crate) fn maybe_prefetch_next(
        mut self: Pin<&mut Self>,
        progress: PlaybackProgress,
        epoch: u64,
    ) {
        if self.play_epoch != epoch || !self.auto_play_next {
            return;
        }
        let limit = prefetch_seconds();
        if limit == 0 {
            return;
        }
        let remaining = progress.duration.saturating_sub(progress.seconds);
        if remaining > limit {
            return;
        }
        let (next, generation) = {
            let mut rust = self.as_mut().rust_mut();
            if rust.play_queue_state.prefetch.is_some()
                || rust.play_queue_state.prefetch_cancel.is_some()
            {
                return;
            }
            let Some(next) = rust.play_queue_state.playlist.next().cloned() else {
                return;
            };
            rust.play_queue_state.prefetch_generation =
                rust.play_queue_state.prefetch_generation.wrapping_add(1);
            (next, rust.play_queue_state.prefetch_generation)
        };
        let cancel = Arc::new(Cancel::default());
        {
            let mut rust = self.as_mut().rust_mut();
            rust.play_queue_state.prefetch_cancel = Some((generation, cancel.clone()));
        }
        let session = self.backend.session.clone();
        let fetch_id = next.id.clone();
        let store_id = next.id.clone();
        let audit_flagged = next.audit_flagged;
        run_in_background(
            move || media::play_options(&session, &fetch_id, Some(&cancel), audit_flagged),
            move |mut qichi, result| {
                let mut rust = qichi.as_mut().rust_mut();
                if rust.play_queue_state.prefetch_generation != generation {
                    return;
                }
                if rust
                    .play_queue_state
                    .prefetch_cancel
                    .as_ref()
                    .is_some_and(|(owner, _)| *owner == generation)
                {
                    rust.play_queue_state.prefetch_cancel = None;
                }
                let Ok(options) = result else {
                    // 预取失败静默丢弃：真正切集时再按需取一次。
                    return;
                };
                if options.is_empty() {
                    return;
                }
                rust.play_queue_state.prefetch = Some(PrefetchedLink {
                    id: store_id,
                    options,
                    fetched_at: Instant::now(),
                });
            },
        );
    }

    /// 取走当前集的预取直链；过期 / 不是这一集就返回 `None`。
    pub(crate) fn take_prefetched(mut self: Pin<&mut Self>, id: &str) -> Option<Vec<PlayOption>> {
        let mut rust = self.as_mut().rust_mut();
        let prefetched = rust.play_queue_state.prefetch.take()?;
        if !prefetch_usable(&prefetched, id, Instant::now()) {
            return None;
        }
        let options = usable_options(prefetched.options);
        (!options.is_empty()).then_some(options)
    }
}

/// 预取结果是否还能用：id 对得上、取到的时间不太久。
pub(super) fn prefetch_usable(prefetched: &PrefetchedLink, id: &str, now: Instant) -> bool {
    prefetched.id == id && now.saturating_duration_since(prefetched.fetched_at) <= PREFETCH_MAX_AGE
}

/// 丢掉已经过期的档位；全过期时返回空，调用方改走按需取链。
pub(super) fn usable_options(options: Vec<PlayOption>) -> Vec<PlayOption> {
    options
        .into_iter()
        .filter(|option| !option.is_expired(None))
        .collect()
}
