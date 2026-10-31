//! 资料库三页的会话入口。
//!
//! 三个功能分支只替换各自子模块的 `load` 实现；页面状态、桥接与视图都共用。

mod playback_history;
mod recent_saved;
mod starred;

use kichi_core::{LibraryEntry, LibraryKind};

use super::{Failure, Session};

impl Session {
    pub(crate) fn load_library(&self, kind: LibraryKind) -> Result<Vec<LibraryEntry>, Failure> {
        match kind {
            LibraryKind::RecentSaved => recent_saved::load(self),
            LibraryKind::PlaybackHistory => playback_history::load(self),
            LibraryKind::Starred => starred::load(self),
        }
    }
}
