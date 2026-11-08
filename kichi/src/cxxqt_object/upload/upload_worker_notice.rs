//! 上传通知与进度节流：批次结束汇总、结构 / 实时刷新回投。
//!
//! 后台线程只把结果打包成 [`UploadEnd`]，由主线程统一合并成一条通知；
//! 进度按首帧、阶段切换、整百分比变化或 250ms 到点回投。

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use kichi_core::{UploadProgress, UploadStage};

use super::transfers;
use super::transfers_sync;
use super::upload_queue;
use super::upload_worker::{same_epoch, BatchOutcome};
use crate::session::Failure;

const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Default)]
pub(crate) struct UploadNoticeGroup {
    done: u32,
    failed: u32,
    cancelled: u32,
    interrupted: u32,
    failure: Option<String>,
}

fn notice_group() -> &'static Mutex<UploadNoticeGroup> {
    static GROUP: OnceLock<Mutex<UploadNoticeGroup>> = OnceLock::new();
    GROUP.get_or_init(|| Mutex::new(UploadNoticeGroup::default()))
}

pub(crate) fn record_upload_end(end: &UploadEnd) {
    let mut group = notice_group()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    match end {
        UploadEnd::Done => group.done = group.done.saturating_add(1),
        UploadEnd::Failed(reason) => {
            group.failed = group.failed.saturating_add(1);
            group.failure = Some(reason.clone());
        }
        UploadEnd::Cancelled => group.cancelled = group.cancelled.saturating_add(1),
        UploadEnd::Interrupted => group.interrupted = group.interrupted.saturating_add(1),
    }
}

pub(crate) fn take_upload_notice() -> Option<super::notifications::Notice> {
    let group = {
        let mut guard = notice_group()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        std::mem::take(&mut *guard)
    };
    super::notifications::upload_finished(super::notifications::UploadSummary {
        done: group.done,
        failed: group.failed,
        cancelled: group.cancelled,
        interrupted: group.interrupted,
        failure: group.failure.as_deref(),
    })
}

pub(crate) fn clear_notice_group() {
    *notice_group()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = UploadNoticeGroup::default();
}

/// 首帧、阶段切换、整百分比变化或 250ms 到点时回投。
#[derive(Default)]
pub(crate) struct ProgressThrottle {
    last_sent: Option<Instant>,
    stage: Option<UploadStage>,
    percent: i32,
    stage_changed: bool,
}

impl ProgressThrottle {
    pub(crate) fn should_send(&mut self, progress: UploadProgress) -> bool {
        let percent = if progress.total > 0 {
            ((progress.bytes as f64 / progress.total as f64) * 100.0) as i32
        } else {
            -1
        };
        let now = Instant::now();
        let first = self.last_sent.is_none();
        let due = self
            .last_sent
            .is_some_and(|last| now.saturating_duration_since(last) >= PROGRESS_INTERVAL);
        let stage_changed = self.stage != Some(progress.stage);
        let changed = stage_changed || self.percent != percent;
        if first || due || changed {
            self.last_sent = Some(now);
            self.stage = Some(progress.stage);
            self.percent = percent;
            self.stage_changed = stage_changed;
            true
        } else {
            self.stage_changed = false;
            false
        }
    }

    pub(crate) fn stage_changed(&self) -> bool {
        self.stage_changed
    }
}

#[derive(Debug)]
pub(crate) enum UploadEnd {
    Done,
    Failed(String),
    Cancelled,
    Interrupted,
}

pub(crate) fn post_finished(batch: u64, outcome: BatchOutcome, my_epoch: u64) {
    super::actions::enqueue_ui(
        move |mut qichi: std::pin::Pin<&mut super::qobject::Kichi>| {
            let current = same_epoch(my_epoch);
            let paused = outcome.paused;
            let kept_paused = current && paused && upload_queue::finish_paused(batch);
            if !kept_paused {
                let _ = upload_queue::release(batch);
            }
            if current {
                if paused {
                    transfers::mark_paused(batch);
                }
                if let Some(parent_id) = outcome.success_parent.as_ref() {
                    qichi.as_mut().on_upload_succeeded(parent_id);
                }
                if let Some(failure) = outcome.fatal {
                    let resets = resets_upload_session(&failure);
                    if resets {
                        qichi.as_mut().reset_uploads();
                    }
                    qichi.as_mut().on_failure(failure);
                }
                if !paused && upload_queue::is_idle() {
                    if let Some(notice) = take_upload_notice() {
                        if qichi.notify_enabled {
                            super::notifications::show_notice(&notice);
                        }
                    }
                }
                qichi.as_mut().sync_transfers();
            }
            qichi.as_mut().advance_uploads();
        },
    );
}

pub(crate) fn post_sync() {
    transfers_sync::request();
}

pub(crate) fn post_live() {
    super::actions::enqueue_ui(
        move |mut qichi: std::pin::Pin<&mut super::qobject::Kichi>| {
            qichi.as_mut().sync_transfer_live();
        },
    );
}

pub(crate) fn fatal_failure(failure: &Failure) -> bool {
    matches!(
        failure,
        Failure::Auth(_) | Failure::RateLimited { .. } | Failure::Captcha { .. }
    )
}

pub(crate) fn resets_upload_session(failure: &Failure) -> bool {
    matches!(failure, Failure::Auth(_))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::cxxqt_object::transfers::TransferJob;
    use crate::cxxqt_object::upload_worker::{UploadFile, UploadStart};
    use crate::cxxqt_object::upload_worker_folder::all_dirs;
    use crate::session::download::Cancel;

    #[test]
    fn throttle_sends_first_change_and_stage_switch() {
        let mut throttle = ProgressThrottle::default();
        assert!(throttle.should_send(UploadProgress {
            stage: UploadStage::Hashing,
            bytes: 0,
            completed_bytes: None,
            total: 1000,
        }));
        assert!(throttle.stage_changed(), "首帧要整表同步阶段名");
        assert!(!throttle.should_send(UploadProgress {
            stage: UploadStage::Hashing,
            bytes: 1,
            completed_bytes: None,
            total: 1000,
        }));
        assert!(!throttle.stage_changed(), "被节流的刷新不能留下阶段变更");
        assert!(throttle.should_send(UploadProgress {
            stage: UploadStage::Hashing,
            bytes: 10,
            completed_bytes: None,
            total: 1000,
        }));
        assert!(!throttle.stage_changed(), "同阶段只走轻量进度刷新");
        assert!(throttle.should_send(UploadProgress {
            stage: UploadStage::Uploading,
            bytes: 20,
            completed_bytes: Some(20),
            total: 1000,
        }));
        assert!(throttle.stage_changed(), "校验转上传必须整表同步");
    }

    #[test]
    fn only_auth_failures_abort_the_remaining_upload_queue() {
        assert!(resets_upload_session(&Failure::Auth("令牌过期".into())));
        assert!(!resets_upload_session(&Failure::RateLimited {
            detail: "配额不足".into()
        }));
        assert!(!resets_upload_session(&Failure::Other("连接中断".into())));
    }

    #[test]
    fn upload_notice_group_aggregates_and_resets() {
        clear_notice_group();
        record_upload_end(&UploadEnd::Done);
        record_upload_end(&UploadEnd::Cancelled);
        let notice = take_upload_notice().expect("有成功要通知");
        assert_eq!(notice.summary, "上传结束");
        assert!(notice.body.contains("成功 1 个，取消 1 个"));
        assert!(take_upload_notice().is_none(), "取走后不能重复弹");
    }

    #[test]
    fn all_dirs_adds_missing_parents_for_retry() {
        let start = UploadStart {
            batch: 1,
            batch_cancel: Arc::new(Cancel::default()),
            parent: Some(TransferJob {
                id: "folder".to_owned(),
                name: "root".to_owned(),
                kind: kichi_core::EntryKind::Folder,
                size: 0,
                audit_flagged: false,
                sub_dir: Vec::new(),
                dir: std::path::PathBuf::new(),
                direction: transfers::TransferDirection::Upload,
                upload: None,
            }),
            files: vec![UploadFile {
                index: 0,
                job: TransferJob {
                    id: "file".to_owned(),
                    name: "a.txt".to_owned(),
                    kind: kichi_core::EntryKind::File,
                    size: 1,
                    audit_flagged: false,
                    sub_dir: vec!["root".to_owned(), "empty".to_owned()],
                    dir: std::path::PathBuf::new(),
                    direction: transfers::TransferDirection::Upload,
                    upload: None,
                },
                cancel: Arc::new(Cancel::default()),
            }]
            .into(),
            dirs: Vec::new().into(),
            upload_dirs: Vec::new().into(),
        };
        assert_eq!(
            all_dirs(&start),
            vec![
                vec!["root".to_owned()],
                vec!["root".to_owned(), "empty".to_owned()]
            ]
        );
    }
}
