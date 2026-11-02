//! 从 mpv 的 `property-change` 事件汇总播放进度。
//!
//! 官方 Web 客户端约每 5 秒上报一次；这里同样按整秒聚合，并在退出时给最后位置。

use std::time::{Duration, Instant};

use kichi_core::PlaybackProgress;

/// 官方 Web 客户端约每 5 秒写一次播放历史。
#[cfg(not(test))]
const PROGRESS_INTERVAL: Duration = Duration::from_secs(5);
/// 测试里缩短间隔，避免每个播放生命周期都白等 5 秒。
#[cfg(test)]
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

pub(super) struct ProgressTracker {
    position: Option<f64>,
    duration: Option<f64>,
    last_emitted: Option<PlaybackProgress>,
    last_emit_at: Instant,
    reached_end: bool,
}

impl ProgressTracker {
    pub(super) fn new(
        position: Option<f64>,
        duration: Option<f64>,
        fallback_duration: Option<u64>,
        now: Instant,
    ) -> Self {
        let mut tracker = Self {
            position: None,
            duration: None,
            last_emitted: None,
            last_emit_at: now,
            reached_end: false,
        };
        tracker.update_values(position, duration);
        if tracker.duration.is_none() {
            tracker.duration = fallback_duration
                .filter(|seconds| *seconds > 0)
                .map(|seconds| seconds as f64);
        }
        tracker
    }

    pub(super) fn update_values(&mut self, position: Option<f64>, duration: Option<f64>) {
        if let Some(position) = valid_position(position) {
            self.position = Some(position);
        }
        if let Some(duration) = valid_duration(duration) {
            self.duration = Some(duration);
        }
    }

    pub(super) fn observe(&mut self, value: &serde_json::Value) {
        let event = value
            .get("event")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if event == "end-file" {
            self.reached_end =
                value.get("reason").and_then(serde_json::Value::as_str) == Some("eof");
            return;
        }
        if event != "property-change" {
            return;
        }
        let number = value
            .get("data")
            .and_then(serde_json::Value::as_f64)
            .filter(|number| number.is_finite());
        match value
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
        {
            "time-pos" => {
                if let Some(position) = valid_position(number) {
                    self.position = Some(position);
                }
            }
            "duration" => {
                if let Some(duration) = valid_duration(number) {
                    self.duration = Some(duration);
                }
            }
            _ => {}
        }
    }

    pub(super) fn maybe_emit(&mut self, now: Instant) -> Option<PlaybackProgress> {
        if now.saturating_duration_since(self.last_emit_at) < PROGRESS_INTERVAL {
            return None;
        }
        self.last_emit_at = now;
        let progress = self.current()?;
        if self.last_emitted == Some(progress) {
            return None;
        }
        self.last_emitted = Some(progress);
        Some(progress)
    }

    pub(super) fn final_progress(&self) -> Option<PlaybackProgress> {
        self.current()
    }

    fn current(&self) -> Option<PlaybackProgress> {
        let duration = self.duration?;
        let duration = duration.floor();
        if duration <= 0.0 {
            return None;
        }
        let duration = duration as u64;
        let last_second = duration.saturating_sub(1);
        if last_second == 0 {
            return None;
        }
        if self.reached_end {
            return Some(PlaybackProgress {
                seconds: last_second,
                duration,
            });
        }
        let position = self.position?;
        let seconds = position.floor().clamp(0.0, last_second as f64);
        if seconds <= 0.0 {
            return None;
        }
        Some(PlaybackProgress {
            seconds: seconds as u64,
            duration,
        })
    }
}

fn valid_position(value: Option<f64>) -> Option<f64> {
    value.filter(|position| position.is_finite() && *position >= 0.0)
}

fn valid_duration(value: Option<f64>) -> Option<f64> {
    value.filter(|duration| duration.is_finite() && *duration > 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_whole_seconds_and_tracks_seeks() {
        let started = Instant::now();
        let mut tracker = ProgressTracker::new(Some(42.9), Some(120.8), None, started);
        assert_eq!(tracker.maybe_emit(started), None, "刚开播不急着写");
        assert_eq!(
            tracker.maybe_emit(started + PROGRESS_INTERVAL),
            Some(PlaybackProgress {
                seconds: 42,
                duration: 120
            })
        );
        assert_eq!(
            tracker.maybe_emit(started + PROGRESS_INTERVAL * 2),
            None,
            "位置没变不重复写"
        );

        tracker.observe(&serde_json::json!({
            "event": "property-change",
            "name": "time-pos",
            "data": 15.2
        }));
        assert_eq!(
            tracker.maybe_emit(started + PROGRESS_INTERVAL * 3),
            Some(PlaybackProgress {
                seconds: 15,
                duration: 120
            }),
            "用户主动回退时也要记录新位置"
        );
        assert_eq!(
            tracker.final_progress(),
            Some(PlaybackProgress {
                seconds: 15,
                duration: 120
            })
        );
    }

    #[test]
    fn ignores_bad_values_and_never_reports_full_duration() {
        let started = Instant::now();
        let mut tracker = ProgressTracker::new(Some(999.0), Some(30.0), None, started);
        assert_eq!(
            tracker.final_progress(),
            Some(PlaybackProgress {
                seconds: 29,
                duration: 30
            })
        );
        tracker.observe(&serde_json::json!({
            "event": "property-change",
            "name": "time-pos",
            "data": null
        }));
        assert_eq!(tracker.final_progress().map(|p| p.seconds), Some(29));

        let none = ProgressTracker::new(Some(-1.0), Some(0.0), None, started);
        assert_eq!(none.final_progress(), None);
    }

    #[test]
    fn only_eof_uses_the_last_second_but_manual_quit_does_not() {
        let started = Instant::now();
        let mut tracker = ProgressTracker::new(Some(0.5), Some(120.8), None, started);
        assert_eq!(tracker.final_progress(), None);

        tracker.observe(&serde_json::json!({
            "event": "end-file",
            "reason": "quit"
        }));
        assert_eq!(tracker.final_progress(), None, "手动关闭不是看完");

        tracker.observe(&serde_json::json!({
            "event": "end-file",
            "reason": "eof"
        }));
        assert_eq!(
            tracker.final_progress(),
            Some(PlaybackProgress {
                seconds: 119,
                duration: 120
            }),
            "自然播放到结尾上报片尾前一秒，由界面显示 100%"
        );
    }
}
