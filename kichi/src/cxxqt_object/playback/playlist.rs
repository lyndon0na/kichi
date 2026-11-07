//! 文件夹连播：队列快照、上一集 / 下一集、片尾预取。
//!
//! 队列只保存目录元数据（id / 名字 / 顺序），不保存任何直链。直链只在
//! 真正要播某一集时取一条；片尾前 [`prefetch_seconds`] 秒最多预取下一集
//! 一条，存内存单槽，过期或换集就丢弃。从 `play.rs` 拆出来守行数。

use std::pin::Pin;
use std::sync::Arc;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QMap, QMapPair_QString_QVariant, QString, QVariant};
use kichi_core::playlist::Playlist;
use kichi_core::types::Tier;
use kichi_core::FileEntry;

use super::actions::run_in_background;
use super::play::fixed_tier_of;
use super::playlist_prefetch::PrefetchedLink;
use super::qobject;
use super::QVariantList;
use crate::player::EndReason;
use crate::session::download::Cancel;

type QVariantMap = QMap<QMapPair_QString_QVariant>;

/// 连播队列的会话状态。
#[derive(Default)]
pub(crate) struct PlayQueueState {
    pub(crate) playlist: Playlist,
    /// 每次新开播 +1；后台列目录回来时代数对不上就丢弃。
    pub(crate) generation: u64,
    /// 当前播放实际使用的档位；上一集 / 下一集沿用，避免每集重新询问。
    pub(crate) current_tier: Option<Tier>,
    pub(crate) prefetch: Option<PrefetchedLink>,
    pub(crate) prefetch_generation: u64,
    /// 在途预取的代数 + 取消信号：只清掉属于自己的那一份。
    pub(crate) prefetch_cancel: Option<(u64, Arc<Cancel>)>,
    pub(crate) listing_cancel: Option<Arc<Cancel>>,
    /// 队列切集时保持内嵌播放窗口不关（避免每集 hide/show 丢全屏）。
    pub(crate) advancing: bool,
    /// 切集前没有仍在播放的内嵌会话：失败 / 取消时要把空窗口收起来。
    pub(crate) hide_on_failure: bool,
}

impl qobject::Kichi {
    /// 开始一次新播放：用当前目录快照建队列；不在当前目录时后台列一次目录。
    ///
    /// 这里只碰元数据，不发任何取直链的请求。
    pub(crate) fn begin_play_queue(mut self: Pin<&mut Self>, id: &str, name: &str) {
        self.as_mut().cancel_queue_work();
        let in_current = self
            .as_ref()
            .rust()
            .entries
            .iter()
            .any(|entry| entry.id == id);
        if in_current {
            let entries = self.as_ref().rust().entries.clone();
            let playlist = Playlist::from_entries(entries, id);
            if playlist.index().is_some() {
                self.as_mut().install_play_queue(playlist);
                return;
            }
        }

        let known = self.as_ref().rust().find_entry(id);
        let folder = self.as_ref().rust().known_folder_id(id);
        let playlist = match known.clone() {
            Some(entry) => Playlist::single(entry),
            None => Playlist::single(synthetic_entry(id, name)),
        };
        let generation = self.as_mut().install_play_queue(playlist);

        // 搜索 / 资料库 / 离线任务：只知道文件本身，后台补一次目录元数据。
        if let Some(folder) = folder {
            self.as_mut()
                .spawn_queue_listing(folder, id.to_owned(), generation, false);
        } else if known.is_none() {
            // 离线任务产物连父目录都不知道：先查父目录再列，仍然只取元数据。
            self.as_mut()
                .spawn_queue_listing(String::new(), id.to_owned(), generation, true);
        }
    }

    /// 安装新队列并推进代数；清掉上一轮的预取。
    fn install_play_queue(mut self: Pin<&mut Self>, playlist: Playlist) -> u64 {
        let generation = {
            let mut rust = self.as_mut().rust_mut();
            rust.play_queue_state.generation = rust.play_queue_state.generation.wrapping_add(1);
            rust.play_queue_state.playlist = playlist;
            rust.play_queue_state.current_tier = None;
            rust.play_queue_state.prefetch = None;
            rust.play_queue_state.prefetch_generation =
                rust.play_queue_state.prefetch_generation.wrapping_add(1);
            if let Some((_, cancel)) = rust.play_queue_state.prefetch_cancel.take() {
                cancel.cancel();
            }
            rust.play_queue_state.generation
        };
        self.as_mut().publish_play_queue();
        generation
    }

    /// 后台列一次目录，回来后只替换队列元数据；播放本身不等它。
    fn spawn_queue_listing(
        mut self: Pin<&mut Self>,
        folder_id: String,
        current_id: String,
        generation: u64,
        needs_parent: bool,
    ) {
        let session = self.backend.session.clone();
        let cancel = Arc::new(Cancel::default());
        {
            let mut rust = self.as_mut().rust_mut();
            if let Some(old) = rust.play_queue_state.listing_cancel.take() {
                old.cancel();
            }
            rust.play_queue_state.listing_cancel = Some(cancel.clone());
        }
        let parent_target = current_id.clone();
        run_in_background(
            move || {
                let folder = if needs_parent {
                    let (_name, parent) = session.parent_of(&parent_target)?;
                    parent
                } else {
                    folder_id
                };
                session.list_dir_with_cancel(
                    (!folder.is_empty()).then_some(folder.as_str()),
                    Some(&cancel),
                )
            },
            move |mut qichi, result| {
                let publish = {
                    let mut rust = qichi.as_mut().rust_mut();
                    if rust.play_queue_state.generation != generation {
                        false
                    } else {
                        rust.play_queue_state.listing_cancel = None;
                        match result {
                            // 列目录失败不影响播放：保持单集队列即可。
                            Err(_) => false,
                            Ok(entries) => {
                                let current = rust
                                    .play_queue_state
                                    .playlist
                                    .current_id()
                                    .map(str::to_owned)
                                    .unwrap_or(current_id);
                                let playlist = Playlist::from_entries(entries, &current);
                                if playlist.index().is_none() {
                                    false
                                } else {
                                    rust.play_queue_state.playlist = playlist;
                                    rust.play_queue_state.prefetch = None;
                                    rust.play_queue_state.prefetch_generation =
                                        rust.play_queue_state.prefetch_generation.wrapping_add(1);
                                    if let Some((_, cancel)) =
                                        rust.play_queue_state.prefetch_cancel.take()
                                    {
                                        cancel.cancel();
                                    }
                                    true
                                }
                            }
                        }
                    }
                };
                if publish {
                    qichi.as_mut().publish_play_queue();
                }
            },
        );
    }

    /// 把队列与当前下标刷给 QML（只有 id / 名字 / 是否当前集）。
    pub(crate) fn publish_play_queue(mut self: Pin<&mut Self>) {
        let (items, index) = {
            let this = self.as_ref();
            let rust = this.rust();
            (
                rust.play_queue_state.playlist.items(),
                rust.play_queue_state.playlist.index(),
            )
        };
        let mut list = QVariantList::default();
        for item in items {
            let mut map = QVariantMap::default();
            map.insert_clone(
                &QString::from("id"),
                &QVariant::from(&QString::from(item.id.as_str())),
            );
            map.insert_clone(
                &QString::from("name"),
                &QVariant::from(&QString::from(item.name.as_str())),
            );
            let episode = kichi_core::episode::episode_label(&item.name);
            map.insert_clone(
                &QString::from("episode"),
                &QVariant::from(&QString::from(
                    episode
                        .as_ref()
                        .map(|episode| episode.text.as_str())
                        .unwrap_or_default(),
                )),
            );
            list.append(QVariant::from(&map));
        }
        {
            let mut rust = self.as_mut().rust_mut();
            rust.play_queue = list;
            rust.play_queue_index = index
                .and_then(|index| i32::try_from(index).ok())
                .unwrap_or(-1);
        }
        self.as_mut().notify();
    }

    /// 只更新当前下标，不重灌队列模型（切集时不打断播放列表的滚动位置）。
    pub(crate) fn publish_play_queue_index(mut self: Pin<&mut Self>) {
        let index = {
            let this = self.as_ref();
            this.rust().play_queue_state.playlist.index()
        };
        {
            let mut rust = self.as_mut().rust_mut();
            rust.play_queue_index = index
                .and_then(|index| i32::try_from(index).ok())
                .unwrap_or(-1);
        }
        self.as_mut().notify();
    }

    /// 播下一集 / 上一集（`offset` 只接受 1 和 -1）。
    pub(crate) fn start_queue_offset(mut self: Pin<&mut Self>, offset: isize) {
        let target = {
            let this = self.as_ref();
            let rust = this.rust();
            match offset {
                1 => rust.play_queue_state.playlist.next().cloned(),
                -1 => rust.play_queue_state.playlist.previous().cloned(),
                _ => None,
            }
        };
        let Some(entry) = target else {
            return;
        };
        self.as_mut().start_queue_entry(entry);
    }

    /// 开始队列里的某一集；档位沿用当前播放，字幕计划从队列快照取。
    pub(crate) fn start_queue_entry(mut self: Pin<&mut Self>, entry: FileEntry) {
        let tier = {
            let mut rust = self.as_mut().rust_mut();
            if !rust.play_queue_state.playlist.select(&entry.id) {
                return;
            }
            // 手动切集也要保持内嵌窗口；只有在播的时候才需要。
            rust.play_queue_state.advancing = rust.embed_active;
            rust.play_queue_state.hide_on_failure = rust.embed_live_epoch.is_none();
            rust.queue_advancing = rust.play_queue_state.advancing;
            // 预取正好是这一集就留着给 fetch_and_play 用；否则丢弃。
            let keep_prefetch = rust
                .play_queue_state
                .prefetch
                .as_ref()
                .is_some_and(|prefetched| prefetched.id == entry.id);
            if !keep_prefetch {
                rust.play_queue_state.prefetch = None;
                rust.play_queue_state.prefetch_generation =
                    rust.play_queue_state.prefetch_generation.wrapping_add(1);
                if let Some((_, cancel)) = rust.play_queue_state.prefetch_cancel.take() {
                    cancel.cancel();
                }
            }
            rust.play_queue_state.current_tier.unwrap_or_else(|| {
                fixed_tier_of(rust.settings.preferred_quality).unwrap_or(Tier::Original)
            })
        };
        self.as_mut().publish_play_queue_index();
        // 下一集的字幕候选直接取队列快照（含字幕文件），不再列一次目录。
        let plan = {
            let this = self.as_ref();
            let rust = this.rust();
            crate::session::subtitles::plan_from_snapshot(
                &entry.name,
                rust.play_queue_state.playlist.entries(),
            )
        };
        self.as_mut().rust_mut().play_subtitle_plan = plan;
        self.as_mut().prepare_play_state(&entry.id, &entry.name);
        self.as_mut().fetch_and_play(tier, &entry.name);
    }

    /// 播放列表里点某一集。
    pub fn play_queue_item(mut self: Pin<&mut Self>, index: i32) {
        let Ok(index) = usize::try_from(index) else {
            return;
        };
        if i32::try_from(index).ok() == Some(self.play_queue_index) {
            return;
        }
        let entry = {
            let mut rust = self.as_mut().rust_mut();
            if !rust.play_queue_state.playlist.select_index(index) {
                return;
            }
            rust.play_queue_state.playlist.current().cloned()
        };
        let Some(entry) = entry else {
            return;
        };
        self.as_mut().start_queue_entry(entry);
    }

    /// 下一集（播放窗口按钮）。
    pub fn play_next(mut self: Pin<&mut Self>) {
        self.as_mut().start_queue_offset(1);
    }

    /// 上一集（播放窗口按钮）。
    pub fn play_previous(mut self: Pin<&mut Self>) {
        self.as_mut().start_queue_offset(-1);
    }

    /// 新开播 / 登出时收掉后台列目录与预取。
    pub(crate) fn cancel_queue_work(mut self: Pin<&mut Self>) {
        let mut rust = self.as_mut().rust_mut();
        if let Some(cancel) = rust.play_queue_state.listing_cancel.take() {
            cancel.cancel();
        }
        if let Some((_, cancel)) = rust.play_queue_state.prefetch_cancel.take() {
            cancel.cancel();
        }
        // 代数一起推进：即使取消信号晚到、线程已经拿到结果，回调也不会再写回。
        rust.play_queue_state.generation = rust.play_queue_state.generation.wrapping_add(1);
        rust.play_queue_state.prefetch = None;
        rust.play_queue_state.prefetch_generation =
            rust.play_queue_state.prefetch_generation.wrapping_add(1);
    }

    /// 退出登录 / 停止播放：清空队列与预取。
    pub(crate) fn reset_play_queue(mut self: Pin<&mut Self>) {
        self.as_mut().cancel_queue_work();
        {
            let mut rust = self.as_mut().rust_mut();
            rust.play_queue_state = PlayQueueState::default();
            rust.play_queue = QVariantList::default();
            rust.play_queue_index = -1;
            rust.embed_serial = 0;
            rust.queue_advancing = false;
        }
        self.as_mut().notify();
    }

    /// 设置页：播完自动下一集。
    pub fn set_auto_play_next(mut self: Pin<&mut Self>, enabled: bool) {
        if self.auto_play_next == enabled {
            return;
        }
        {
            let mut rust = self.as_mut().rust_mut();
            rust.auto_play_next = enabled;
        }
        self.as_mut().update_settings(|settings| {
            settings.auto_play_next = enabled;
        });
    }
}

/// 从搜索结果 / 资料库 / 当前目录推断文件所在文件夹；不知道时为空。
impl super::KichiRust {
    pub(crate) fn known_folder_id(&self, id: &str) -> Option<String> {
        if let Some(hit) = self.search_index.get(id) {
            return Some(hit.folder_id.clone());
        }
        if let Some(entry) = self
            .library_entries
            .iter()
            .map(|entry| &entry.entry)
            .find(|entry| entry.id == id)
        {
            return Some(entry.parent_id.clone());
        }
        self.entries
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| entry.parent_id.clone())
    }
}

/// 片尾是否该自动切下一集。
pub(super) fn should_auto_advance(end: EndReason, enabled: bool, has_next: bool) -> bool {
    enabled && has_next && end == EndReason::Eof
}

/// 离线任务只给了 id / 名字：构造一个只用于队列显示的条目。
fn synthetic_entry(id: &str, name: &str) -> FileEntry {
    FileEntry {
        id: id.to_owned(),
        name: name.to_owned(),
        kind: kichi_core::EntryKind::File,
        size: 0,
        created_at: None,
        modified_at: None,
        parent_id: String::new(),
        md5: None,
        audit_flagged: false,
        starred: false,
        media: kichi_core::is_media_name(name),
        video: kichi_core::is_video_name(name),
        thumbnail_url: String::new(),
    }
}

#[cfg(test)]
#[path = "playlist_tests.rs"]
mod tests;
