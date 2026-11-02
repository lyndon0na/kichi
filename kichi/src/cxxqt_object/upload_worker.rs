//! 上传线程：取得会话 client、上传单个文件、节流回投进度。
//!
//! 后台线程绝不碰 Qt 对象；结构变化和实时进度分别经 `enqueue_ui` 回主线程。

use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use kichi_core::{UploadProgress, UploadResult, UploadStage};

use super::transfers::{self, TransferJob, UploadProgressState};
use super::upload_queue;
use crate::session::download::Cancel;
use crate::session::{Failure, Session};

const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Default)]
struct UploadNoticeGroup {
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

fn record_upload_end(end: &UploadEnd) {
    let mut group = notice_group()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    match end {
        UploadEnd::Done => group.done = group.done.saturating_add(1),
        UploadEnd::Failed(failure) => {
            group.failed = group.failed.saturating_add(1);
            group.failure = Some(failure.to_string());
        }
        UploadEnd::Cancelled => group.cancelled = group.cancelled.saturating_add(1),
        UploadEnd::Interrupted => group.interrupted = group.interrupted.saturating_add(1),
    }
}

fn take_upload_notice() -> Option<super::notifications::Notice> {
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

pub(crate) struct UploadStart {
    pub(crate) batch: u64,
    pub(crate) job: TransferJob,
    pub(crate) cancel: Arc<Cancel>,
}

pub(crate) fn spawn_upload(session: &Arc<Session>, start: &UploadStart) -> std::io::Result<()> {
    let session = Arc::clone(session);
    let batch = start.batch;
    let job = start.job.clone();
    let cancel = Arc::clone(&start.cancel);
    let my_epoch = upload_queue::current_epoch();
    std::thread::Builder::new()
        .name("kichi-upload".into())
        .spawn(move || run(session, batch, job, cancel, my_epoch))
        .map(|_| ())
}

fn run(session: Arc<Session>, batch: u64, job: TransferJob, cancel: Arc<Cancel>, my_epoch: u64) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        upload_one(&session, batch, &job, &cancel)
    }));
    let same_epoch = my_epoch == upload_queue::current_epoch();
    let end = match result {
        Ok(Ok(result)) => {
            if same_epoch {
                transfers::finish_upload_ok(batch, 0, &result);
            }
            UploadEnd::Done
        }
        Ok(Err(_)) if !same_epoch || cancel.is_abandoned() => UploadEnd::Interrupted,
        Ok(Err(_)) if cancel.is_cancelled() => {
            if same_epoch {
                transfers::cancel_batch(batch, "");
            }
            UploadEnd::Cancelled
        }
        Ok(Err(err)) => {
            let reason = err.to_string();
            if same_epoch {
                transfers::finish_err(batch, 0, &reason);
            }
            UploadEnd::Failed(err)
        }
        Err(_) => {
            let reason = "上传线程异常中断".to_owned();
            if same_epoch {
                transfers::finish_err(batch, 0, &reason);
            }
            UploadEnd::Failed(Failure::Other(reason))
        }
    };
    post_finished(batch, job, end, my_epoch);
}

fn upload_one(
    session: &Arc<Session>,
    batch: u64,
    job: &TransferJob,
    cancel: &Arc<Cancel>,
) -> Result<UploadResult, Failure> {
    let upload = job
        .upload
        .as_ref()
        .ok_or_else(|| Failure::Other("上传任务缺少本地文件信息".to_owned()))?;
    // 先拿并持有 client 再上传：登出清掉会话令牌后，收尾清理仍可用同一实例。
    let client = session.drive()?;
    let mut throttle = ProgressThrottle::default();
    session.upload_file(
        client,
        &upload.local_path,
        &upload.parent_id,
        Arc::clone(cancel),
        |progress| {
            if throttle.should_send(progress) {
                transfers::upload_progress(
                    batch,
                    0,
                    UploadProgressState {
                        stage: progress.stage,
                        bytes: progress.bytes,
                        total: progress.total,
                    },
                );
                post_live();
            }
        },
    )
}

/// 首帧、阶段切换、整百分比变化或 250ms 到点时回投。
#[derive(Default)]
struct ProgressThrottle {
    last_sent: Option<Instant>,
    stage: Option<UploadStage>,
    percent: i32,
}

impl ProgressThrottle {
    fn should_send(&mut self, progress: UploadProgress) -> bool {
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
        let changed = self.stage != Some(progress.stage) || self.percent != percent;
        if first || due || changed {
            self.last_sent = Some(now);
            self.stage = Some(progress.stage);
            self.percent = percent;
            true
        } else {
            false
        }
    }
}

#[derive(Debug)]
enum UploadEnd {
    Done,
    Failed(Failure),
    Cancelled,
    Interrupted,
}

fn post_finished(batch: u64, job: TransferJob, end: UploadEnd, my_epoch: u64) {
    super::actions::enqueue_ui(
        move |mut qichi: std::pin::Pin<&mut super::qobject::Kichi>| {
            let current = my_epoch == upload_queue::current_epoch();
            let _ = upload_queue::release(batch);
            if current {
                record_upload_end(&end);
                match end {
                    UploadEnd::Done => {
                        if let Some(upload) = job.upload.as_ref() {
                            qichi.as_mut().on_upload_succeeded(&upload.parent_id);
                        }
                    }
                    UploadEnd::Failed(failure) => match failure {
                        failure @ (Failure::Auth(_) | Failure::RateLimited { .. }) => {
                            if resets_upload_session(&failure) {
                                // 认证失败后继续启动后面的文件只会反复失败，并可能使用旧账号上下文。
                                qichi.as_mut().reset_uploads();
                            }
                            qichi.as_mut().on_failure(failure);
                        }
                        failure => {
                            eprintln!("[kichi] 上传失败（{}）：{failure}", job.name);
                        }
                    },
                    UploadEnd::Cancelled | UploadEnd::Interrupted => {}
                }
                if upload_queue::is_idle() {
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

fn post_live() {
    super::actions::enqueue_ui(
        move |mut qichi: std::pin::Pin<&mut super::qobject::Kichi>| {
            qichi.as_mut().sync_transfer_live();
        },
    );
}

fn resets_upload_session(failure: &Failure) -> bool {
    matches!(failure, Failure::Auth(_))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn throttle_sends_first_change_and_stage_switch() {
        let mut throttle = ProgressThrottle::default();
        assert!(throttle.should_send(UploadProgress {
            stage: UploadStage::Hashing,
            bytes: 0,
            total: 1000,
        }));
        assert!(!throttle.should_send(UploadProgress {
            stage: UploadStage::Hashing,
            bytes: 1,
            total: 1000,
        }));
        assert!(throttle.should_send(UploadProgress {
            stage: UploadStage::Hashing,
            bytes: 10,
            total: 1000,
        }));
        assert!(throttle.should_send(UploadProgress {
            stage: UploadStage::Uploading,
            bytes: 20,
            total: 1000,
        }));
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
}
