//! 资料库三页的会话入口。
//!
//! 三个功能分支只替换各自子模块的 `load` 实现；页面状态、桥接与视图都共用。
//! `status` 是对用户交代数据来源的一句话（星标页在本地索引与云端之间切换），
//! 其余页面保持空串。

mod playback_history;
mod recent_saved;
mod starred;

use kichi_core::{LibraryEntry, LibraryKind};

use super::{page_snapshot, Failure, Session};

/// 一次资料库读取：条目 + 数据来源说明。
pub(crate) struct LibraryLoad {
    pub(crate) entries: Vec<LibraryEntry>,
    pub(crate) status: String,
}

impl LibraryLoad {
    pub(crate) fn new(entries: Vec<LibraryEntry>, status: String) -> Self {
        Self { entries, status }
    }

    /// 没有额外来源说明的页面。
    pub(crate) fn quiet(entries: Vec<LibraryEntry>) -> Self {
        Self {
            entries,
            status: String::new(),
        }
    }
}

impl Session {
    pub(crate) fn load_library(&self, kind: LibraryKind) -> Result<LibraryLoad, Failure> {
        let load = match kind {
            LibraryKind::RecentSaved => recent_saved::load(self).map(LibraryLoad::quiet),
            LibraryKind::PlaybackHistory => playback_history::load(self).map(LibraryLoad::quiet),
            LibraryKind::Starred => starred::load(self),
        }?;
        if let Some(key) = page_snapshot::library_key(kind) {
            page_snapshot::save(self, key, &load.entries);
        }
        Ok(load)
    }
}
