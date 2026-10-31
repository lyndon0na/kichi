//! 官方播放历史：查找续播位置、写入播放进度。
//!
//! 阻塞调用，线程与界面状态由 `cxxqt_object::play` 负责。

use std::collections::HashSet;

use kichi_core::events::PLAY_HISTORY_LIMIT;
use kichi_core::PlaybackProgress;

use super::download::Cancel;
use super::{Failure, Session};

/// 最多翻 20 页（2,000 条）。历史按最近播放排序，目标通常在第一页；
/// 上限只防异常分页，不在打开视频时无限扫描账号活动。
const MAX_PLAY_HISTORY_PAGES: usize = 20;

impl Session {
    /// 查找某个文件的最近播放位置。找不到返回 `None`。
    pub fn playback_progress(
        &self,
        file_id: &str,
        cancel: Option<&Cancel>,
    ) -> Result<Option<PlaybackProgress>, Failure> {
        let file_id = file_id.trim();
        if file_id.is_empty() {
            return Ok(None);
        }
        let client = self.drive()?;
        let mut page_token: Option<String> = None;
        let mut seen = HashSet::new();
        for _ in 0..MAX_PLAY_HISTORY_PAGES {
            if cancel.is_some_and(Cancel::is_cancelled) {
                return Ok(None);
            }
            let (events, next) = client
                .list_play_history(page_token.as_deref())
                .map_err(Failure::from)?;
            if let Some(progress) = events
                .into_iter()
                .find(|entry| entry.file_id == file_id)
                .and_then(|entry| entry.progress)
            {
                return Ok(Some(progress));
            }
            match next {
                Some(token) if seen.insert(token.clone()) => page_token = Some(token),
                Some(_) => {
                    return Err(Failure::Other(
                        "播放历史分页出现循环，已停止继续读取".into(),
                    ));
                }
                None => return Ok(None),
            }
        }
        eprintln!(
            "[kichi] 播放历史超过 {} 条仍未找到目标，本次从头播放",
            MAX_PLAY_HISTORY_PAGES * PLAY_HISTORY_LIMIT as usize
        );
        Ok(None)
    }

    /// 把当前播放位置写入官方播放历史（阻塞）。
    pub fn report_playback_progress(
        &self,
        file_id: &str,
        progress: PlaybackProgress,
    ) -> Result<(), Failure> {
        let client = self.drive()?;
        client.report_play(file_id, progress).map_err(Failure::from)
    }
}
