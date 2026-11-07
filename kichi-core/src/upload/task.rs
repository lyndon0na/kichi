//! 上传服务端任务的轮询与取消后的尽力清理。

use std::time::Instant;

use reqwest::Method;
use serde_json::json;

use crate::drive::DriveClient;
use crate::http::Body as ApiBody;
use crate::upload_state::{UploadError, UploadResume};
use crate::{Config, Error};

use super::s3::S3Target;
use super::types::{TaskDto, UploadCleanup};
use super::util::{
    ensure_not_cancelled, is_retryable_api_error, keep_resume_for_api_error, non_empty,
    preserve_after_error, sleep_cancellable,
};
use super::{CLEANUP_TIMEOUT, TASK_POLL_INTERVAL, TASK_TIMEOUT, UPLOAD_MAX_ATTEMPTS};

impl DriveClient {
    pub(super) fn wait_upload_task<C>(
        &self,
        task_id: &str,
        cancelled: &C,
    ) -> Result<TaskDto, UploadError>
    where
        C: Fn() -> bool + Clone + Send + Sync + 'static,
    {
        let started = Instant::now();
        let mut transient_failures = 0u32;
        loop {
            ensure_not_cancelled(cancelled).map_err(UploadError::fatal)?;
            let task: TaskDto = match self.get(&Config::task_url(self.config(), task_id)) {
                Ok(task) => {
                    transient_failures = 0;
                    task
                }
                Err(error)
                    if transient_failures + 1 < UPLOAD_MAX_ATTEMPTS
                        && is_retryable_api_error(&error) =>
                {
                    transient_failures += 1;
                    sleep_cancellable(TASK_POLL_INTERVAL, cancelled).map_err(UploadError::fatal)?;
                    continue;
                }
                Err(error) if keep_resume_for_api_error(&error) => {
                    return Err(preserve_after_error(error))
                }
                Err(error) => return Err(UploadError::fatal(error)),
            };
            match task.phase.as_str() {
                "PHASE_TYPE_COMPLETE" => return Ok(task),
                "PHASE_TYPE_ERROR" => {
                    let reason =
                        non_empty(task.message).unwrap_or_else(|| "服务端未能完成上传".to_owned());
                    return Err(UploadError::fatal(Error::Other(reason)));
                }
                _ => {}
            }
            if started.elapsed() >= TASK_TIMEOUT {
                return Err(UploadError::retryable(Error::Other(
                    "上传已提交，但服务端任务长时间没有完成".to_owned(),
                )));
            }
            sleep_cancellable(TASK_POLL_INTERVAL, cancelled).map_err(UploadError::fatal)?;
        }
    }

    /// 尽力清理一次上传在服务端留下的临时状态。
    ///
    /// 用户取消时由会话层在后台线程调用；失败只代表残留可能还在，不能改变
    /// “已取消”的结果。
    pub fn cleanup_upload(&self, state: &UploadResume) {
        self.cleanup_upload_inner(&UploadCleanup {
            file_id: state.file_id.clone(),
            task_id: state.task_id.clone(),
            upload_id: state.upload_id.clone(),
            credentials: Some(state.credentials.clone()),
            content_uploaded: state.content_uploaded,
        });
    }

    pub(super) fn cleanup_upload_residue(&self, cleanup: &UploadCleanup) {
        self.cleanup_upload_inner(cleanup);
    }

    pub(super) fn cleanup_upload_inner(&self, cleanup: &UploadCleanup) {
        if !cleanup.content_uploaded && !cleanup.upload_id.trim().is_empty() {
            if let Some(credentials) = cleanup.credentials.as_ref() {
                if let Ok(target) =
                    S3Target::new_with_timeout(self.config(), credentials, None, CLEANUP_TIMEOUT)
                {
                    let _ = target.abort_multipart(cleanup.upload_id.trim());
                }
            }
        }
        if !cleanup.file_id.trim().is_empty() {
            let body = json!({"ids": [cleanup.file_id]});
            let _ = self.request::<serde_json::Value>(
                Method::POST,
                &Config::file_action_url(self.config(), "batchDelete"),
                &[],
                ApiBody::Json(&body),
            );
        }
        if !cleanup.task_id.trim().is_empty() {
            let query = vec![
                ("delete_files".to_owned(), "false".to_owned()),
                ("task_ids".to_owned(), cleanup.task_id.clone()),
            ];
            let _ = self.request_no_response(Method::DELETE, &self.config().tasks_url(), &query);
        }
    }
}
