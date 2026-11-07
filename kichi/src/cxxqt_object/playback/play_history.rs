//! 把 mpv 观察到的播放位置串行写入官方播放历史。
//!
//! 同一文件同一时间只允许一个 POST 在途：mpv 每 5 秒给一次样本，网络可能更慢，
//! 并发上报会让旧位置后到并覆盖新位置。每个文件只保留最新待发样本，旧请求结束
//! 后再发；退出登录会清空队列，并用播放代数隔离旧请求的回投。

use std::collections::{HashMap, HashSet};
use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;
use kichi_core::PlaybackProgress;

use super::actions::run_in_background;
use super::qobject;
use crate::session::Failure;

/// 这次播放要不要写官方播放历史。
///
/// 只记视频：音乐 / 音频不往账号写 `TYPE_PLAY` 事件，避免听歌记录挤进
/// 官方播放历史；历史读取仍照旧，音频有旧记录时仍可续播。
pub(super) fn reports_progress(name: &str) -> bool {
    kichi_core::is_video_name(name)
}

/// 播放历史上报状态。纯数据结构，便于锁定同一文件串行与旧回投隔离。
#[derive(Default)]
pub(crate) struct ProgressReporter {
    pending: HashMap<String, (u64, PlaybackProgress)>,
    inflight: HashMap<String, u64>,
    alerted_epochs: HashSet<u64>,
    /// 退出登录时记下最后作废的播放代数；旧 mpv 之后回投的进度不能写进新账号。
    blocked_through_epoch: u64,
}

impl ProgressReporter {
    fn enqueue(&mut self, id: &str, epoch: u64, progress: PlaybackProgress) -> bool {
        if epoch <= self.blocked_through_epoch {
            return false;
        }
        let Some(progress) = progress.normalized() else {
            return false;
        };
        self.pending.insert(id.to_owned(), (epoch, progress));
        true
    }

    fn take_next(&mut self, id: &str) -> Option<(u64, PlaybackProgress)> {
        if self.inflight.contains_key(id) {
            return None;
        }
        let (epoch, progress) = self.pending.remove(id)?;
        self.inflight.insert(id.to_owned(), epoch);
        Some((epoch, progress))
    }

    /// 只有当前这条请求仍属于该文件时才能解除占用。
    ///
    /// 退出登录会 `clear()`；重新登录后同一文件可能已经发起了新请求。旧请求晚回时
    /// epoch 对不上，不能误删新请求的 inflight 标记。
    fn finish(&mut self, id: &str, epoch: u64) -> bool {
        if self.inflight.get(id) != Some(&epoch) {
            return false;
        }
        self.inflight.remove(id);
        true
    }

    fn mark_error(&mut self, epoch: u64) -> bool {
        self.alerted_epochs.insert(epoch)
    }

    pub(super) fn reset(&mut self, through_epoch: u64) {
        self.pending.clear();
        self.inflight.clear();
        self.alerted_epochs.clear();
        self.blocked_through_epoch = through_epoch;
    }

    #[cfg(test)]
    fn is_inflight(&self, id: &str) -> bool {
        self.inflight.contains_key(id)
    }
}

impl qobject::Kichi {
    pub(super) fn queue_play_progress(
        mut self: Pin<&mut Self>,
        id: &str,
        epoch: u64,
        progress: PlaybackProgress,
    ) {
        let queued = self
            .as_mut()
            .rust_mut()
            .play_progress
            .enqueue(id, epoch, progress);
        if queued {
            self.as_mut().dispatch_play_progress(id);
        }
    }

    fn dispatch_play_progress(mut self: Pin<&mut Self>, id: &str) {
        let Some((epoch, progress)) = self.as_mut().rust_mut().play_progress.take_next(id) else {
            return;
        };
        let session = self.backend.session.clone();
        let file_id = id.to_owned();
        let callback_id = id.to_owned();
        run_in_background(
            move || session.report_playback_progress(&file_id, progress),
            move |mut qichi, result| {
                qichi
                    .as_mut()
                    .finish_play_progress_report(&callback_id, epoch, result);
            },
        );
    }

    fn finish_play_progress_report(
        mut self: Pin<&mut Self>,
        id: &str,
        epoch: u64,
        result: Result<(), Failure>,
    ) {
        let dispatch_next;
        let mut show_notice = false;
        {
            let mut rust = self.as_mut().rust_mut();
            dispatch_next = rust.play_progress.finish(id, epoch);
            if let Err(err) = result {
                eprintln!("[kichi] 保存播放记录失败（不影响播放）：{err}");
                if rust.play_epoch == epoch && rust.play_progress.mark_error(epoch) {
                    let existing =
                        (!rust.play_notice.is_empty()).then(|| rust.play_notice.to_string());
                    rust.play_notice = QString::from(
                        super::play_prepare::combine_notes(
                            existing,
                            Some("播放记录暂时无法保存，本次播放不受影响".to_owned()),
                        )
                        .unwrap_or_default()
                        .as_str(),
                    );
                    show_notice = true;
                }
            }
        }
        if dispatch_next {
            self.as_mut().dispatch_play_progress(id);
        }
        if show_notice {
            self.as_mut().notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn progress(seconds: u64) -> PlaybackProgress {
        PlaybackProgress {
            seconds,
            duration: 120,
        }
    }

    #[test]
    fn one_file_never_has_two_reports_in_flight() {
        let mut reporter = ProgressReporter::default();
        assert!(reporter.enqueue("f", 1, progress(5)));
        assert_eq!(reporter.take_next("f"), Some((1, progress(5))));
        assert!(reporter.is_inflight("f"));

        assert!(reporter.enqueue("f", 1, progress(10)));
        assert_eq!(reporter.take_next("f"), None, "第一条没回完不能再发");
        assert!(reporter.finish("f", 1));
        assert_eq!(
            reporter.take_next("f"),
            Some((1, progress(10))),
            "只发该文件最新样本"
        );
    }

    #[test]
    fn a_stale_callback_cannot_clear_a_new_play_request() {
        let mut reporter = ProgressReporter::default();
        assert!(reporter.enqueue("f", 1, progress(10)));
        assert_eq!(reporter.take_next("f"), Some((1, progress(10))));

        reporter.reset(1);
        assert!(
            !reporter.enqueue("f", 1, progress(15)),
            "旧账号进度必须丢弃"
        );
        assert!(reporter.enqueue("f", 2, progress(20)));
        assert_eq!(reporter.take_next("f"), Some((2, progress(20))));

        assert!(!reporter.finish("f", 1), "旧代数的回投必须丢弃");
        assert!(reporter.is_inflight("f"), "不能误删新请求的占用");
        assert!(reporter.finish("f", 2));
    }

    #[test]
    fn only_video_playback_is_reported_to_history() {
        assert!(reports_progress("Movie.mkv"));
        assert!(reports_progress("Show.S01E01.MP4"));
        assert!(!reports_progress("song.flac"));
        assert!(!reports_progress("track.mp3"));
        assert!(!reports_progress("audiobook.m4b"));
        assert!(!reports_progress("README"));
    }

    #[test]
    fn zero_progress_is_ignored_and_errors_alert_once_per_play() {
        let mut reporter = ProgressReporter::default();
        assert!(!reporter.enqueue("f", 1, progress(0)));
        assert!(reporter.mark_error(1));
        assert!(!reporter.mark_error(1));
        assert!(reporter.mark_error(2));
    }
}
