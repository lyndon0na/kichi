//! `playlist.rs` / `playlist_prefetch.rs` 的单元测试：队列与预取的纯逻辑。

use std::time::{Duration, Instant};

use super::{should_auto_advance, synthetic_entry};
use crate::cxxqt_object::playlist_prefetch::{
    prefetch_usable, usable_options, PrefetchedLink, PREFETCH_MAX_AGE,
};
use crate::player::EndReason;
use kichi_core::types::{StreamKind, Tier};
use kichi_core::PlayOption;

fn option(url: &str) -> PlayOption {
    PlayOption {
        label: "原画".to_owned(),
        url: url.to_owned(),
        kind: StreamKind::Original,
        height: None,
        original: true,
        tier: Some(Tier::Original),
        available: true,
        reason: None,
    }
}

fn link(id: &str, fetched_at: Instant) -> PrefetchedLink {
    PrefetchedLink {
        id: id.to_owned(),
        options: vec![option("https://cdn.invalid/x")],
        fetched_at,
    }
}

#[test]
fn prefetch_is_only_usable_for_the_same_episode_and_recent_enough() {
    let now = Instant::now();
    let prefetched = link("a", now);
    assert!(prefetch_usable(&prefetched, "a", now));
    assert!(!prefetch_usable(&prefetched, "b", now));
    assert!(prefetch_usable(
        &prefetched,
        "a",
        now + Duration::from_secs(60)
    ));
    assert!(!prefetch_usable(
        &prefetched,
        "a",
        now + PREFETCH_MAX_AGE + Duration::from_secs(1)
    ));
}

#[test]
fn expired_prefetched_links_are_dropped() {
    let now = chrono::Utc::now().timestamp();
    let expired = format!("https://cdn.invalid/x?expire={}", now - 60);
    let valid = format!("https://cdn.invalid/x?expire={}", now + 3600);
    let options = usable_options(vec![option(&expired), option(&valid)]);
    assert_eq!(options.len(), 1);
    assert!(options[0].url.contains("expire="));
    assert!(!options[0].is_expired(None));
    assert!(usable_options(vec![option(&expired)]).is_empty());
}

#[test]
fn only_eof_with_a_next_episode_advances() {
    assert!(should_auto_advance(EndReason::Eof, true, true));
    assert!(!should_auto_advance(EndReason::Stopped, true, true));
    assert!(!should_auto_advance(EndReason::Unknown, true, true));
    assert!(!should_auto_advance(EndReason::Eof, false, true));
    assert!(!should_auto_advance(EndReason::Eof, true, false));
}

#[test]
fn synthetic_entry_marks_media_by_extension() {
    assert!(synthetic_entry("1", "第1集.mkv").media);
    assert!(!synthetic_entry("2", "说明.txt").media);
}
