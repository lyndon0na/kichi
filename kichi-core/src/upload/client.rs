//! 上传入口：连接池、断点身份与单文件上传编排。

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use reqwest::blocking::Client;
use reqwest::Method;
use serde_json::json;

use crate::drive::DriveClient;
use crate::http::Body as ApiBody;
use crate::naming::{duplicate_file_name, MAX_DUPLICATE_ATTEMPTS};
use crate::upload_rate::UploadRateLimiter;
use crate::upload_state::{UploadError, UploadResume};
use crate::{Config, Error};

use super::types::{UploadCleanup, UploadTicket};
use super::util::{
    ensure_file_size, ensure_not_cancelled, local_file_info, non_empty_name, resume_matches,
    validate_name, LocalFileInfo,
};
use super::{UploadProgress, UploadResult, UploadStage, S3_REQUEST_TIMEOUT};

impl DriveClient {
    /// 上传专用 client：所有上传文件共用一个连接池，避免每个文件重建
    /// client 导致反复 TCP/TLS 握手。超时语义是单次 S3 请求最多 5 分钟。
    pub(super) fn upload_client(&self) -> Result<Arc<Client>, Error> {
        {
            let cached = self.upload.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(client) = cached.as_ref() {
                return Ok(Arc::clone(client));
            }
        }
        // 锁外构建：reqwest blocking client 会起一条专属运行时线程。
        let built = Arc::new(
            Client::builder()
                .user_agent(&self.config().user_agent)
                .connect_timeout(Duration::from_secs(15))
                .timeout(S3_REQUEST_TIMEOUT)
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| Error::Network("无法创建上传 HTTP 客户端".to_owned()))?,
        );
        let mut cached = self.upload.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(client) = cached.as_ref() {
            return Ok(Arc::clone(client));
        }
        *cached = Some(Arc::clone(&built));
        Ok(built)
    }

    /// 把本地普通文件上传到 `parent_id`（空串 = 根目录）。
    ///
    /// 这是不带断点的兼容入口；需要跨进程续传时用
    /// [`DriveClient::upload_file_resumable`]。
    pub fn upload_file<C, P>(
        &self,
        path: &Path,
        parent_id: &str,
        cancelled: C,
        progress: P,
    ) -> Result<UploadResult, Error>
    where
        C: Fn() -> bool + Clone + Send + Sync + 'static,
        P: FnMut(UploadProgress),
    {
        self.upload_file_resumable(path, parent_id, cancelled, progress, None, None, |_| {})
            .map_err(|failure| failure.error)
    }

    /// 上传文件，并允许带上一轮留下的 multipart 断点。
    ///
    /// `checkpoint` 会在拿到票据、创建 multipart、每个分片完成和内容提交
    /// 完成时被调用。调用方应把它写进 KWallet（或同进程缓存）；`None`
    /// 表示当前没有可复用断点。
    #[allow(clippy::too_many_arguments)]
    pub fn upload_file_resumable<C, P, F>(
        &self,
        path: &Path,
        parent_id: &str,
        cancelled: C,
        mut progress: P,
        resume: Option<UploadResume>,
        limiter: Option<Arc<UploadRateLimiter>>,
        mut checkpoint: F,
    ) -> Result<UploadResult, UploadError>
    where
        C: Fn() -> bool + Clone + Send + Sync + 'static,
        P: FnMut(UploadProgress),
        F: FnMut(&UploadResume),
    {
        let local = local_file_info(path)?;
        let raw_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| Error::InvalidInput("文件名不是有效的 UTF-8".to_owned()))?;
        if raw_name.trim().is_empty() {
            return Err(Error::InvalidInput("文件名不能为空".to_owned()).into());
        }
        let name = raw_name.to_owned();
        validate_name(&name)?;

        let mut resume = resume;
        if let Some(state) = resume.as_ref() {
            if !resume_matches(state, &local, parent_id) {
                // 断点对应的本地文件已经变化，不能拿旧分片拼一份新文件。
                // 已经提交内容的上传可能是完整文件，别自动删；只丢弃断点。
                if !state.content_uploaded {
                    self.cleanup_upload_residue(&UploadCleanup {
                        file_id: state.file_id.clone(),
                        task_id: state.task_id.clone(),
                        upload_id: state.upload_id.clone(),
                        credentials: Some(state.credentials.clone()),
                        content_uploaded: state.content_uploaded,
                    });
                }
                resume = None;
            }
        }

        let mut cleanup = UploadCleanup::default();
        let outcome = self.upload_file_inner(
            path,
            &local,
            &name,
            parent_id,
            &cancelled,
            &mut progress,
            resume,
            limiter.as_ref(),
            &mut checkpoint,
            &mut cleanup,
        );
        match outcome {
            Ok(result) => Ok(result),
            Err(failure) => {
                // 用户取消和登出都会让 `cancelled()` 为真；取消后的清理由
                // 会话层按“用户取消 / 登出保留”分别决定，core 不在这里猜。
                if !failure.keep_resume && !cancelled() {
                    self.cleanup_upload_residue(&cleanup);
                }
                Err(failure)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn upload_file_inner<C, P, F>(
        &self,
        path: &Path,
        local: &LocalFileInfo,
        name: &str,
        parent_id: &str,
        cancelled: &C,
        progress: &mut P,
        resume: Option<UploadResume>,
        limiter: Option<&Arc<UploadRateLimiter>>,
        checkpoint: &mut F,
        cleanup: &mut UploadCleanup,
    ) -> Result<UploadResult, UploadError>
    where
        C: Fn() -> bool + Clone + Send + Sync + 'static,
        P: FnMut(UploadProgress),
        F: FnMut(&UploadResume),
    {
        let size = local.size;
        progress(UploadProgress {
            stage: UploadStage::Preparing,
            bytes: 0,
            completed_bytes: None,
            total: size,
        });

        let mut state = if let Some(state) = resume {
            cleanup.file_id = state.file_id.clone();
            cleanup.task_id = state.task_id.clone();
            cleanup.upload_id = state.upload_id.clone();
            cleanup.credentials = Some(state.credentials.clone());
            cleanup.content_uploaded = state.content_uploaded;
            state
        } else {
            let gcid = self
                .gcid_for_file(path, size, cancelled, progress)
                .map_err(UploadError::fatal)?;
            ensure_not_cancelled(cancelled).map_err(UploadError::fatal)?;
            ensure_file_size(path, size).map_err(UploadError::fatal)?;

            progress(UploadProgress {
                stage: UploadStage::Preparing,
                bytes: 0,
                completed_bytes: None,
                total: size,
            });
            let requested_name;
            let mut duplicate_attempt = 0u32;
            let ticket: UploadTicket = loop {
                let candidate = duplicate_file_name(name, duplicate_attempt);
                let ticket_body = json!({
                    "kind": "drive#file",
                    "name": candidate.clone(),
                    "parent_id": parent_id.trim(),
                    "folder_type": "NORMAL",
                    "size": size,
                    "hash": gcid.to_ascii_uppercase(),
                    "upload_type": "UPLOAD_TYPE_RESUMABLE",
                    "resumable": {"provider": "PROVIDER_ALIYUN"}
                });
                match self.request(
                    Method::POST,
                    &Config::files_url(self.config()),
                    &[],
                    ApiBody::Json(&ticket_body),
                ) {
                    Ok(ticket) => {
                        requested_name = candidate;
                        break ticket;
                    }
                    Err(Error::DuplicateName) if duplicate_attempt + 1 < MAX_DUPLICATE_ATTEMPTS => {
                        duplicate_attempt += 1;
                    }
                    Err(err) => return Err(UploadError::fatal(err)),
                }
            };
            cleanup.file_id = ticket.file.id.trim().to_owned();
            cleanup.task_id = ticket
                .task
                .as_ref()
                .map(|task| task.id.trim().to_owned())
                .unwrap_or_default();
            if cleanup.file_id.is_empty() {
                return Err(UploadError::fatal(Error::ApiChanged(
                    "上传票据缺少文件 id".to_owned(),
                )));
            }

            let final_name =
                non_empty_name(ticket.file.name.clone()).unwrap_or_else(|| requested_name.clone());
            if ticket.file.phase == "PHASE_TYPE_COMPLETE" {
                return Ok(UploadResult {
                    file_id: ticket.file.id,
                    name: final_name,
                    bytes_sent: 0,
                });
            }

            let credentials = ticket.resumable.params.ok_or_else(|| {
                UploadError::fatal(Error::ApiChanged("上传票据缺少 S3 临时凭据".to_owned()))
            })?;
            credentials.validate().map_err(UploadError::fatal)?;
            cleanup.credentials = Some(credentials.clone());
            let state = UploadResume::new(
                size,
                local.modified_unix_secs,
                local.modified_nanos,
                parent_id.trim().to_owned(),
                cleanup.file_id.clone(),
                cleanup.task_id.clone(),
                gcid,
                requested_name,
                final_name,
                credentials,
            );
            checkpoint(&state);
            state
        };

        if state.content_uploaded {
            progress(UploadProgress {
                stage: UploadStage::Finalizing,
                bytes: size,
                completed_bytes: Some(size),
                total: size,
            });
            return self.finish_upload_task(&mut state, 0, cancelled, checkpoint);
        }

        if state.upload_id.is_empty() {
            state.bytes_sent = 0;
            state.parts.clear();
            state.part_size = 0;
        }
        let transfer = self.transfer_to_s3(
            path, size, &mut state, cancelled, progress, limiter, checkpoint,
        );
        let (completed, sent) = match transfer {
            Ok(result) => result,
            Err(error) => {
                cleanup.upload_id = state.upload_id.clone();
                cleanup.credentials = Some(state.credentials.clone());
                cleanup.content_uploaded = state.content_uploaded;
                return Err(error);
            }
        };
        state.bytes_sent = completed;
        state.content_uploaded = true;
        checkpoint(&state);

        if let Err(err) = ensure_file_size(path, size) {
            return Err(UploadError::fatal(err));
        }
        progress(UploadProgress {
            stage: UploadStage::Finalizing,
            bytes: size,
            completed_bytes: Some(size),
            total: size,
        });
        match self.finish_upload_task(&mut state, sent, cancelled, checkpoint) {
            Ok(result) => Ok(result),
            Err(error) => {
                cleanup.file_id = state.file_id.clone();
                cleanup.task_id = state.task_id.clone();
                cleanup.content_uploaded = state.content_uploaded;
                Err(error)
            }
        }
    }

    pub(super) fn finish_upload_task<C, F>(
        &self,
        state: &mut UploadResume,
        bytes_sent: u64,
        cancelled: &C,
        checkpoint: &mut F,
    ) -> Result<UploadResult, UploadError>
    where
        C: Fn() -> bool + Clone + Send + Sync + 'static,
        F: FnMut(&UploadResume),
    {
        if !state.task_id.trim().is_empty() {
            let task = self.wait_upload_task(&state.task_id, cancelled)?;
            if !task.file_id.trim().is_empty() {
                state.file_id = task.file_id;
            }
            if let Some(name) = non_empty_name(task.file_name) {
                state.final_name = name;
            }
            state.touch();
            checkpoint(state);
        }
        Ok(UploadResult {
            file_id: state.file_id.clone(),
            name: if state.final_name.trim().is_empty() {
                state.requested_name.clone()
            } else {
                state.final_name.clone()
            },
            bytes_sent,
        })
    }
}
