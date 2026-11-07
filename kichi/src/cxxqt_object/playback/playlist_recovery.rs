//! 播放失败 / 取消后的队列恢复：旧视频还在播时，把队列切回它。
//!
//! 从 `playlist.rs` 拆出来守行数。典型场景：正在看 B，点了 A 的播放又取消 /
//! 失败；B 仍在播，但队列已经被 A 替换。这里按仍在播的文件 id 从当前目录重建，
//! 找不到就退回单集。

use std::pin::Pin;

use cxx_qt::CxxQtType;
use kichi_core::playlist::Playlist;

use super::qobject;

impl qobject::Kichi {
    /// 新播放取链失败 / 取消时，如果旧视频还在播，把队列恢复到旧视频。
    ///
    /// 典型场景：正在看 B，点了 A 的播放又取消 / 失败；B 仍在播，但队列已经被
    /// A 替换。这里按仍在播的文件 id 从当前目录重建，找不到就退回单集。
    pub(crate) fn restore_queue_for_live_session(mut self: Pin<&mut Self>) {
        let (live_id, needs_restore) = {
            let this = self.as_ref();
            let rust = this.rust();
            if rust.embed_live_epoch.is_none() {
                return;
            }
            let Some(live_id) = rust.active_play_sessions.values().next_back().cloned() else {
                return;
            };
            let needs = rust.play_queue_state.playlist.current_id() != Some(live_id.as_str());
            (live_id, needs)
        };
        if !needs_restore {
            return;
        }
        // 队列确实要换回旧视频：顺手作废新播放留下的列目录 / 预取结果。
        self.as_mut().cancel_queue_work();
        let playlist = {
            let this = self.as_ref();
            let rust = this.rust();
            let from_folder = Playlist::from_entries(rust.entries.clone(), &live_id);
            if from_folder.index().is_some() {
                from_folder
            } else if let Some(entry) = rust.find_entry(&live_id) {
                Playlist::single(entry)
            } else {
                return;
            }
        };
        {
            let mut rust = self.as_mut().rust_mut();
            rust.play_queue_state.playlist = playlist;
            rust.play_queue_state.prefetch = None;
            rust.play_queue_state.prefetch_generation =
                rust.play_queue_state.prefetch_generation.wrapping_add(1);
            if let Some((_, cancel)) = rust.play_queue_state.prefetch_cancel.take() {
                cancel.cancel();
            }
        }
        self.as_mut().publish_play_queue();
    }
}
