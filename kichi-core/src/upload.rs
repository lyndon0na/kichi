//! 上传：CID / GCID、上传票据、S3 SigV4、单段 / 分片上传与服务端任务确认。
//!
//! 上传票据里的临时 access key、secret 和 security token 只活在本模块调用栈内，
//! 不进入日志、错误文案或任何持久化结构。

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, UNIX_EPOCH};

use chrono::Utc;
use hmac::{Hmac, KeyInit, Mac};
use quick_xml::events::Event;
use quick_xml::Reader;
use reqwest::blocking::{Body, Client};
use reqwest::header::{CONTENT_LENGTH, CONTENT_TYPE, ETAG, HOST};
use reqwest::{Method, StatusCode, Url};
use serde::Deserialize;
use serde_json::json;
use sha1::{Digest as Sha1Digest, Sha1};
use sha2::{Digest as Sha256Digest, Sha256};

use crate::drive::DriveClient;
use crate::http::Body as ApiBody;
use crate::naming::{duplicate_file_name, MAX_DUPLICATE_ATTEMPTS, MAX_NAME_BYTES};
use crate::upload_rate::UploadRateLimiter;
use crate::upload_state::{
    UploadCredentials, UploadError, UploadPart, UploadResume, UPLOAD_RESUME_VERSION,
};
use crate::{Config, Error};

const SMALL_FILE_LIMIT: u64 = 5 * 1024 * 1024;
const MAX_MULTIPART_PARTS: u64 = 10_000;
const CID_SAMPLE_BYTES: u64 = 20 * 1024;
const CID_FULL_FILE_LIMIT: u64 = 0xF000;
const TASK_POLL_INTERVAL: Duration = Duration::from_secs(1);
const TASK_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_ERROR_BODY: u64 = 64 * 1024;
const UPLOAD_MAX_ATTEMPTS: u32 = 3;
const UPLOAD_RETRY_BASE: Duration = Duration::from_millis(500);
const UPLOAD_RETRY_MAX: Duration = Duration::from_secs(4);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);

/// 上传阶段。准备阶段也可能包含完整的 GCID 回退计算。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadStage {
    Preparing,
    Hashing,
    Uploading,
    Finalizing,
}

/// 一次上传进度。`total` 是当前阶段的总字节数（未知时为 0）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UploadProgress {
    pub stage: UploadStage,
    pub bytes: u64,
    pub total: u64,
}

/// 上传成功后的服务端信息。文件名可能被官方自动追加编号。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadResult {
    pub file_id: String,
    pub name: String,
    pub bytes_sent: u64,
}

struct LocalFileInfo {
    size: u64,
    modified_unix_secs: i64,
    modified_nanos: u32,
}

impl DriveClient {
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
    fn upload_file_inner<C, P, F>(
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

    fn finish_upload_task<C, F>(
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

    fn gcid_for_file<C, P>(
        &self,
        path: &Path,
        size: u64,
        cancelled: &C,
        progress: &mut P,
    ) -> Result<String, Error>
    where
        C: Fn() -> bool + Clone + Send + Sync + 'static,
        P: FnMut(UploadProgress),
    {
        let sampled_total = if size <= CID_FULL_FILE_LIMIT {
            size
        } else {
            CID_SAMPLE_BYTES * 3
        };
        progress(UploadProgress {
            stage: UploadStage::Hashing,
            bytes: 0,
            total: sampled_total,
        });
        let cid = cid_for_file(path, size, cancelled, progress)?;
        ensure_not_cancelled(cancelled)?;
        if size == 0 {
            return Ok(cid);
        }

        let query = vec![
            ("cid".to_owned(), cid.clone()),
            ("file_size".to_owned(), size.to_string()),
        ];
        match self.get_with_query::<CidResponse>(&self.config().resource_cid_url(), &query) {
            Ok(response) => {
                let gcid = response.gcid.trim();
                if !gcid.is_empty() {
                    return Ok(gcid.to_owned());
                }
            }
            Err(err) if is_hard_auth_error(&err) => return Err(err),
            Err(_) => {}
        }
        full_gcid_for_file(path, size, cancelled, progress)
    }

    #[allow(clippy::too_many_arguments)]
    fn transfer_to_s3<C, P, F>(
        &self,
        path: &Path,
        size: u64,
        state: &mut UploadResume,
        cancelled: &C,
        progress: &mut P,
        limiter: Option<&Arc<UploadRateLimiter>>,
        checkpoint: &mut F,
    ) -> Result<(u64, u64), UploadError>
    where
        C: Fn() -> bool + Clone + Send + Sync + 'static,
        P: FnMut(UploadProgress),
        F: FnMut(&UploadResume),
    {
        ensure_file_size(path, size)?;
        let target = S3Target::new(self.config(), &state.credentials, limiter.cloned())?;
        if size <= SMALL_FILE_LIMIT {
            if state.content_uploaded {
                return Ok((state.bytes_sent, 0));
            }
            let mut file =
                File::open(path).map_err(|err| Error::Other(format!("无法打开本地文件：{err}")))?;
            progress(UploadProgress {
                stage: UploadStage::Hashing,
                bytes: 0,
                total: size,
            });
            let digest = sha256_hex(&mut file, cancelled, progress, size)?;
            let payload_hash = hex_lower(&digest);
            progress(UploadProgress {
                stage: UploadStage::Uploading,
                bytes: 0,
                total: size,
            });
            drop(file);
            target
                .put_object(
                    path,
                    size,
                    &payload_hash,
                    "application/octet-stream",
                    cancelled,
                )
                .map_err(|failure| UploadError {
                    error: failure.error,
                    // 单次 PUT 没有可续传的分片；失败只能整文件重来。
                    keep_resume: false,
                    stop_queue: failure.stop_queue,
                })?;
            state.bytes_sent = size;
            state.content_uploaded = true;
            checkpoint(state);
            progress(UploadProgress {
                stage: UploadStage::Uploading,
                bytes: size,
                total: size,
            });
            return Ok((size, size));
        }

        let resuming = !state.upload_id.is_empty();
        if state.upload_id.is_empty() {
            state.upload_id = target.create_multipart(cancelled)?;
            state.part_size = multipart_part_size(size);
            state.parts.clear();
            state.bytes_sent = 0;
            checkpoint(state);
        }
        let part_size = state.part_size;
        if part_size == 0 || part_size < SMALL_FILE_LIMIT {
            return Err(UploadError::fatal(Error::InvalidInput(
                "上传断点的分片大小无效，请重新开始".to_owned(),
            )));
        }
        let part_count = size.div_ceil(part_size);
        if part_count == 0 || part_count > MAX_MULTIPART_PARTS {
            return Err(UploadError::fatal(Error::InvalidInput(
                "上传断点的分片数量无效，请重新开始".to_owned(),
            )));
        }

        let mut completed: BTreeMap<u64, UploadPart> = BTreeMap::new();
        if resuming {
            match target.list_parts(&state.upload_id, cancelled) {
                Ok(parts) => {
                    for part in parts {
                        if !valid_completed_part(&part, part_count, part_size, size) {
                            return Err(UploadError::fatal(Error::Other(
                                "上传断点的分片信息无效，请重新开始".to_owned(),
                            )));
                        }
                        completed.insert(part.number, part);
                    }
                }
                Err(ListPartsError::NoSuchUpload) => {
                    let exists = target.object_exists(size, cancelled)?;
                    if exists {
                        state.content_uploaded = true;
                        state.bytes_sent = size;
                        state.touch();
                        checkpoint(state);
                        return Ok((size, 0));
                    }
                    return Err(UploadError::fatal(Error::Other(
                        "上传断点已失效，请重新开始".to_owned(),
                    )));
                }
                Err(ListPartsError::Other(error)) => return Err(error),
            }
        }
        state.parts = completed.values().cloned().collect();
        state.bytes_sent = completed_part_bytes(&completed, part_size, size);
        state.touch();
        progress(UploadProgress {
            stage: UploadStage::Uploading,
            bytes: state.bytes_sent,
            total: size,
        });

        let mut sent_this_run = 0u64;
        for part_number in 1..=part_count {
            if completed.contains_key(&part_number) {
                continue;
            }
            ensure_not_cancelled(cancelled)?;
            ensure_file_size(path, size)?;
            let offset = (part_number - 1) * part_size;
            let length = part_length(part_number, part_size, size);
            let bytes = Arc::new(read_exact_range(path, offset, length, cancelled)?);
            let digest = Sha256::digest(bytes.as_slice());
            let etag = target.upload_part(
                &state.upload_id,
                part_number,
                length,
                &hex_lower(&digest),
                bytes,
                cancelled,
            )?;
            completed.insert(
                part_number,
                UploadPart {
                    number: part_number,
                    etag,
                    size: length,
                },
            );
            sent_this_run = sent_this_run.saturating_add(length);
            state.parts = completed.values().cloned().collect();
            state.bytes_sent = completed_part_bytes(&completed, part_size, size);
            state.touch();
            checkpoint(state);
            progress(UploadProgress {
                stage: UploadStage::Uploading,
                bytes: state.bytes_sent,
                total: size,
            });
        }
        ensure_file_size(path, size)?;
        target.complete_multipart(&state.upload_id, &state.parts, cancelled)?;
        state.content_uploaded = true;
        state.touch();
        checkpoint(state);
        Ok((state.bytes_sent, sent_this_run))
    }

    fn wait_upload_task<C>(&self, task_id: &str, cancelled: &C) -> Result<TaskDto, UploadError>
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

    fn cleanup_upload_residue(&self, cleanup: &UploadCleanup) {
        self.cleanup_upload_inner(cleanup);
    }

    fn cleanup_upload_inner(&self, cleanup: &UploadCleanup) {
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

#[derive(Debug, Deserialize, Default)]
struct CidResponse {
    #[serde(default)]
    gcid: String,
}

#[derive(Debug, Deserialize, Default)]
struct UploadTicket {
    #[serde(default)]
    file: TicketFile,
    #[serde(default)]
    resumable: TicketResumable,
    #[serde(default)]
    task: Option<TaskDto>,
}

#[derive(Debug, Deserialize, Default)]
struct TicketFile {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    phase: String,
}

#[derive(Debug, Deserialize, Default)]
struct TicketResumable {
    #[serde(default)]
    params: Option<UploadCredentials>,
}

#[derive(Debug, Deserialize, Default)]
struct TaskDto {
    #[serde(default)]
    id: String,
    #[serde(default)]
    phase: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    file_id: String,
    #[serde(default)]
    file_name: String,
}

#[derive(Debug, Default, Clone)]
struct UploadCleanup {
    file_id: String,
    task_id: String,
    upload_id: String,
    credentials: Option<UploadCredentials>,
    content_uploaded: bool,
}

struct S3Target {
    client: Client,
    credentials: UploadCredentials,
    host: String,
    path: String,
    limiter: Option<Arc<UploadRateLimiter>>,
}

impl S3Target {
    fn new(
        config: &Config,
        credentials: &UploadCredentials,
        limiter: Option<Arc<UploadRateLimiter>>,
    ) -> Result<Self, Error> {
        Self::new_with_timeout(config, credentials, limiter, Duration::from_secs(300))
    }

    fn new_with_timeout(
        config: &Config,
        credentials: &UploadCredentials,
        limiter: Option<Arc<UploadRateLimiter>>,
        timeout: Duration,
    ) -> Result<Self, Error> {
        let base = Url::parse(&config.upload_base)
            .map_err(|_| Error::InvalidInput("上传基地址无效".to_owned()))?;
        let base_host = base
            .host_str()
            .map(|host| host.trim_end_matches('.').to_ascii_lowercase())
            .ok_or_else(|| Error::InvalidInput("上传基地址缺少主机名".to_owned()))?;
        if base.scheme() != "https"
            || !(base_host == "mypikpak.com" || base_host.ends_with(".mypikpak.com"))
        {
            return Err(Error::InvalidInput(
                "上传基地址只允许 HTTPS mypikpak.com 或子域".to_owned(),
            ));
        }
        let bucket = credentials.bucket.trim().to_ascii_lowercase();
        let host = format!("{bucket}.{base_host}");
        let client = Client::builder()
            .user_agent(&config.user_agent)
            .connect_timeout(Duration::from_secs(15))
            // 单次 S3 请求最坏 5 分钟；不是整文件的固定总分片时限。
            .timeout(timeout)
            // 上传请求只允许直连收窄后的 mypikpak.com 主机，不能跟去别处。
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| Error::Network("无法创建上传 HTTP 客户端".to_owned()))?;
        Ok(Self {
            client,
            credentials: credentials.clone(),
            host,
            path: encode_path(credentials.key.trim()),
            limiter,
        })
    }

    fn put_object<C>(
        &self,
        path: &Path,
        size: u64,
        payload_hash: &str,
        content_type: &str,
        cancelled: &C,
    ) -> Result<(), UploadError>
    where
        C: Fn() -> bool + Clone + Send + Sync + 'static,
    {
        retry_upload(cancelled, |_attempt| {
            ensure_file_size(path, size).map_err(UploadAttemptError::Fatal)?;
            let file = File::open(path).map_err(|err| {
                UploadAttemptError::Fatal(Error::Other(format!("无法打开本地文件：{err}")))
            })?;
            let body = RateLimitedReader::new(
                CancelReader::new(file, cancelled.clone()),
                self.limiter.clone(),
                cancelled.clone(),
            );
            let response = self
                .send(
                    Method::PUT,
                    &[],
                    payload_hash,
                    S3RequestBody {
                        content_length: (size > 0).then_some(size),
                        content_type: Some(content_type),
                        body: Some(Body::sized(body, size)),
                    },
                    cancelled,
                )
                .map_err(UploadAttemptError::Retryable)?;
            if response.status.is_success() {
                Ok(())
            } else {
                Err(s3_attempt_error(response.status, &response.body))
            }
        })
    }

    fn create_multipart<C>(&self, cancelled: &C) -> Result<String, UploadError>
    where
        C: Fn() -> bool + Clone + Send + Sync + 'static,
    {
        ensure_not_cancelled(cancelled)?;
        let empty_hash = hex_lower(&Sha256::digest([]));
        let response = self
            .send(
                Method::POST,
                &[("uploads", String::new())],
                &empty_hash,
                S3RequestBody::default(),
                cancelled,
            )
            .map_err(preserve_after_error)?;
        if !response.status.is_success() {
            return Err(upload_error_from_s3(response.status, &response.body));
        }
        if parse_s3_error_body(&response.body).is_some() {
            return Err(upload_error_from_s3(response.status, &response.body));
        }
        xml_text(response.body.as_bytes(), b"UploadId")
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                UploadError::fatal(Error::ApiChanged("分片上传响应缺少 UploadId".to_owned()))
            })
    }

    fn list_parts<C>(
        &self,
        upload_id: &str,
        cancelled: &C,
    ) -> Result<Vec<UploadPart>, ListPartsError>
    where
        C: Fn() -> bool + Clone + Send + Sync + 'static,
    {
        let empty_hash = hex_lower(&Sha256::digest([]));
        let mut parts = Vec::new();
        let mut marker: Option<u64> = None;
        for _ in 0..=MAX_MULTIPART_PARTS.div_ceil(500) {
            let mut query: Vec<(&str, String)> = vec![
                ("uploadId", upload_id.to_owned()),
                ("max-parts", "500".to_owned()),
            ];
            if let Some(marker) = marker {
                query.push(("part-number-marker", marker.to_string()));
            }
            let response = self
                .send(
                    Method::GET,
                    &query,
                    &empty_hash,
                    S3RequestBody::default(),
                    cancelled,
                )
                .map_err(preserve_after_error)
                .map_err(ListPartsError::Other)?;
            if !response.status.is_success() {
                if parse_s3_error_body(&response.body)
                    .is_some_and(|(code, _)| code == "NoSuchUpload")
                {
                    return Err(ListPartsError::NoSuchUpload);
                }
                return Err(ListPartsError::Other(upload_error_from_s3(
                    response.status,
                    &response.body,
                )));
            }
            let page = parse_list_parts(&response.body)
                .map_err(UploadError::fatal)
                .map_err(ListPartsError::Other)?;
            parts.extend(page.parts);
            if !page.truncated {
                return Ok(parts);
            }
            marker = page.next_marker;
            if marker.is_none() {
                return Err(ListPartsError::Other(UploadError::fatal(
                    Error::ApiChanged("分片列表分页缺少下一标记".to_owned()),
                )));
            }
        }
        Err(ListPartsError::Other(UploadError::fatal(
            Error::ApiChanged("分片列表超过服务端上限".to_owned()),
        )))
    }

    fn upload_part<C>(
        &self,
        upload_id: &str,
        part_number: u64,
        size: u64,
        payload_hash: &str,
        bytes: Arc<Vec<u8>>,
        cancelled: &C,
    ) -> Result<String, UploadError>
    where
        C: Fn() -> bool + Clone + Send + Sync + 'static,
    {
        retry_upload(cancelled, |_attempt| {
            let body = RateLimitedReader::new(
                CancelReader::new(ByteSliceReader::new(Arc::clone(&bytes)), cancelled.clone()),
                self.limiter.clone(),
                cancelled.clone(),
            );
            let response = self
                .send(
                    Method::PUT,
                    &[
                        ("partNumber", part_number.to_string()),
                        ("uploadId", upload_id.to_owned()),
                    ],
                    payload_hash,
                    S3RequestBody {
                        content_length: Some(size),
                        content_type: Some("application/octet-stream"),
                        body: Some(Body::sized(body, size)),
                    },
                    cancelled,
                )
                .map_err(UploadAttemptError::Retryable)?;
            if !response.status.is_success() {
                return Err(s3_attempt_error(response.status, &response.body));
            }
            response
                .etag
                .filter(|etag| !etag.trim().is_empty())
                .map(|etag| etag.trim().to_owned())
                .ok_or_else(|| {
                    UploadAttemptError::Fatal(Error::ApiChanged("上传分片响应缺少 ETag".to_owned()))
                })
        })
    }

    fn complete_multipart<C>(
        &self,
        upload_id: &str,
        parts: &[UploadPart],
        cancelled: &C,
    ) -> Result<(), UploadError>
    where
        C: Fn() -> bool + Clone + Send + Sync + 'static,
    {
        let mut sorted: Vec<&UploadPart> = parts.iter().collect();
        sorted.sort_by_key(|part| part.number);
        let mut xml = String::from("<CompleteMultipartUpload>");
        for part in sorted {
            xml.push_str("<Part><PartNumber>");
            xml.push_str(&part.number.to_string());
            xml.push_str("</PartNumber><ETag>");
            xml.push_str(&xml_escape(&part.etag));
            xml.push_str("</ETag></Part>");
        }
        xml.push_str("</CompleteMultipartUpload>");
        let bytes = Arc::new(xml.into_bytes());
        let payload_hash = hex_lower(&Sha256::digest(bytes.as_slice()));
        let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        retry_upload(cancelled, |_attempt| {
            let body =
                CancelReader::new(ByteSliceReader::new(Arc::clone(&bytes)), cancelled.clone());
            let response = self
                .send(
                    Method::POST,
                    &[("uploadId", upload_id.to_owned())],
                    &payload_hash,
                    S3RequestBody {
                        content_length: Some(size),
                        content_type: Some("application/xml"),
                        body: Some(Body::sized(body, size)),
                    },
                    cancelled,
                )
                .map_err(UploadAttemptError::Retryable)?;
            if let Some((code, _message)) = parse_s3_error_body(&response.body) {
                // Complete 的响应丢失或重试时，服务端可能已经完成并让会话失效。
                // 先 HEAD 对象；大小匹配才算成功，不能把别的同名对象当结果。
                if code == "NoSuchUpload" {
                    let exists = self
                        .object_exists(size, cancelled)
                        .map_err(UploadAttemptError::Preserve)?;
                    if exists {
                        return Ok(());
                    }
                }
                if response.status.is_success() {
                    return Err(s3_attempt_error(response.status, &response.body));
                }
            } else if response.status.is_success() {
                return Ok(());
            }
            Err(s3_attempt_error(response.status, &response.body))
        })
    }

    fn object_exists<C>(&self, size: u64, cancelled: &C) -> Result<bool, UploadError>
    where
        C: Fn() -> bool + Clone + Send + Sync + 'static,
    {
        ensure_not_cancelled(cancelled).map_err(UploadError::fatal)?;
        let empty_hash = hex_lower(&Sha256::digest([]));
        let response = self
            .send(
                Method::HEAD,
                &[],
                &empty_hash,
                S3RequestBody::default(),
                cancelled,
            )
            .map_err(preserve_after_error)?;
        if response.status == StatusCode::NOT_FOUND {
            return Ok(false);
        }
        if !response.status.is_success() {
            return Err(upload_error_from_s3(response.status, &response.body));
        }
        Ok(response.content_length.is_none_or(|actual| actual == size))
    }

    fn abort_multipart(&self, upload_id: &str) -> Result<(), Error> {
        let empty_hash = hex_lower(&Sha256::digest([]));
        let response = self.send(
            Method::DELETE,
            &[("uploadId", upload_id.to_owned())],
            &empty_hash,
            S3RequestBody::default(),
            &|| false,
        )?;
        ensure_s3_success(response.status, &response.body)
    }

    fn send<C>(
        &self,
        method: Method,
        query: &[(&str, String)],
        payload_hash: &str,
        body: S3RequestBody<'_>,
        cancelled: &C,
    ) -> Result<S3Response, Error>
    where
        C: Fn() -> bool + Clone + Send + Sync + 'static,
    {
        ensure_not_cancelled(cancelled)?;
        let mut url = Url::parse(&format!("https://{}{}", self.host, self.path))
            .map_err(|_| Error::InvalidInput("上传对象地址无效".to_owned()))?;
        {
            let mut pairs = url.query_pairs_mut();
            for (key, value) in query {
                pairs.append_pair(key, value);
            }
        }

        let now = Utc::now();
        let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
        let short_date = now.format("%Y%m%d").to_string();
        let signed = sign_request(
            method.as_str(),
            url.path(),
            query,
            &self.host,
            body.content_length,
            body.content_type,
            &self.credentials.security_token,
            payload_hash,
            &amz_date,
            &short_date,
            "pikpak",
            "s3",
            &self.credentials.access_key_id,
            &self.credentials.access_key_secret,
        )?;

        let mut request = self
            .client
            .request(method, url)
            .header(HOST, &self.host)
            .header("x-amz-content-sha256", payload_hash)
            .header("x-amz-date", &amz_date)
            .header("Authorization", signed);
        if let Some(length) = body.content_length {
            request = request.header(reqwest::header::CONTENT_LENGTH, length.to_string());
        }
        if let Some(content_type) = body.content_type {
            request = request.header(CONTENT_TYPE, content_type);
        }
        if !self.credentials.security_token.trim().is_empty() {
            request = request.header(
                "x-amz-security-token",
                self.credentials.security_token.trim(),
            );
        }
        if let Some(body) = body.body {
            request = request.body(body);
        }
        let response = request.send().map_err(s3_network_error)?;
        let status = response.status();
        let etag = response
            .headers()
            .get(ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let content_length = response
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok());
        let body = read_limited(response, MAX_ERROR_BODY)?;
        Ok(S3Response {
            status,
            etag,
            content_length,
            body,
        })
    }
}

#[derive(Default)]
struct S3RequestBody<'a> {
    content_length: Option<u64>,
    content_type: Option<&'a str>,
    body: Option<Body>,
}

struct S3Response {
    status: StatusCode,
    etag: Option<String>,
    content_length: Option<u64>,
    body: String,
}

enum UploadAttemptError {
    Retryable(Error),
    Fatal(Error),
    /// 不自动重试，但保留断点并停住本轮上传队列。
    Hard(Error),
    /// 探测请求已经带回了完整的上传错误分类，直接原样上抛。
    Preserve(UploadError),
}

enum ListPartsError {
    NoSuchUpload,
    Other(UploadError),
}

struct ListPartsPage {
    parts: Vec<UploadPart>,
    truncated: bool,
    next_marker: Option<u64>,
}

/// 重放可安全重复的 S3 操作。票据申请和 CreateMultipartUpload 不经过这里，
/// 避免响应丢失时在云端留下两个请求各自创建的上传记录。
fn retry_upload<T, C, F>(cancelled: &C, mut operation: F) -> Result<T, UploadError>
where
    C: Fn() -> bool,
    F: FnMut(u32) -> Result<T, UploadAttemptError>,
{
    let mut attempt = 0u32;
    loop {
        ensure_not_cancelled(cancelled).map_err(UploadError::fatal)?;
        match operation(attempt) {
            Ok(value) => return Ok(value),
            Err(UploadAttemptError::Fatal(error)) => return Err(UploadError::fatal(error)),
            Err(UploadAttemptError::Hard(error)) => return Err(UploadError::hard(error)),
            Err(UploadAttemptError::Preserve(error)) => return Err(error),
            Err(UploadAttemptError::Retryable(error)) => {
                if attempt + 1 >= UPLOAD_MAX_ATTEMPTS {
                    return Err(if is_hard_auth_error(&error) {
                        UploadError::hard(error)
                    } else {
                        UploadError::retryable(error)
                    });
                }
                sleep_cancellable(retry_delay(attempt), cancelled).map_err(UploadError::fatal)?;
                attempt += 1;
            }
        }
    }
}

fn retry_delay(attempt: u32) -> Duration {
    let factor = 1u32 << attempt.min(4);
    UPLOAD_RETRY_BASE
        .saturating_mul(factor)
        .min(UPLOAD_RETRY_MAX)
}

fn s3_attempt_error(status: StatusCode, body: &str) -> UploadAttemptError {
    let parsed = parse_s3_error_body(body);
    if is_hard_s3_response(status, parsed.as_ref().map(|(code, _)| code.as_str())) {
        return UploadAttemptError::Hard(s3_error(status, body));
    }
    let retryable = retryable_status(status)
        || parsed
            .as_ref()
            .is_some_and(|(code, _)| retryable_s3_code(code));
    let error = s3_error(status, body);
    if retryable {
        UploadAttemptError::Retryable(error)
    } else {
        UploadAttemptError::Fatal(error)
    }
}

fn upload_error_from_s3(status: StatusCode, body: &str) -> UploadError {
    let parsed = parse_s3_error_body(body);
    let error = s3_error(status, body);
    if is_hard_s3_response(status, parsed.as_ref().map(|(code, _)| code.as_str())) {
        UploadError::hard(error)
    } else {
        UploadError::fatal(error)
    }
}

fn is_hard_s3_response(status: StatusCode, code: Option<&str>) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS
        || matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN)
        || code.is_some_and(|code| {
            code == "QuotaExceeded"
                || matches!(
                    code,
                    "AccessDenied"
                        | "ExpiredToken"
                        | "InvalidAccessKeyId"
                        | "SignatureDoesNotMatch"
                        | "TokenRefreshRequired"
                )
        })
}

fn retryable_status(status: StatusCode) -> bool {
    matches!(status.as_u16(), 408 | 425 | 500 | 502 | 503 | 504)
}

fn retryable_s3_code(code: &str) -> bool {
    matches!(
        code,
        "RequestTimeout" | "InternalError" | "ServiceUnavailable" | "SlowDown"
    )
}

struct ByteSliceReader {
    bytes: Arc<Vec<u8>>,
    offset: usize,
}

impl ByteSliceReader {
    fn new(bytes: Arc<Vec<u8>>) -> Self {
        Self { bytes, offset: 0 }
    }
}

impl Read for ByteSliceReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let remaining = self.bytes.len().saturating_sub(self.offset);
        if remaining == 0 || buffer.is_empty() {
            return Ok(0);
        }
        let length = remaining.min(buffer.len());
        buffer[..length].copy_from_slice(&self.bytes[self.offset..self.offset + length]);
        self.offset += length;
        Ok(length)
    }
}

#[allow(clippy::too_many_arguments)]
fn sign_request(
    method: &str,
    canonical_uri: &str,
    query: &[(&str, String)],
    host: &str,
    content_length: Option<u64>,
    content_type: Option<&str>,
    security_token: &str,
    payload_hash: &str,
    amz_date: &str,
    short_date: &str,
    region: &str,
    service: &str,
    access_key: &str,
    secret_key: &str,
) -> Result<String, Error> {
    let mut headers = BTreeMap::<String, String>::new();
    headers.insert("host".to_owned(), normalize_header(host));
    if let Some(length) = content_length {
        headers.insert("content-length".to_owned(), length.to_string());
    }
    if let Some(content_type) = content_type {
        headers.insert("content-type".to_owned(), normalize_header(content_type));
    }
    headers.insert("x-amz-content-sha256".to_owned(), payload_hash.to_owned());
    headers.insert("x-amz-date".to_owned(), amz_date.to_owned());
    if !security_token.trim().is_empty() {
        headers.insert(
            "x-amz-security-token".to_owned(),
            normalize_header(security_token),
        );
    }
    let signed_headers = headers.keys().cloned().collect::<Vec<_>>().join(";");
    let canonical_headers = headers
        .iter()
        .map(|(key, value)| format!("{key}:{value}\n"))
        .collect::<String>();
    let canonical_query = canonical_query(query);
    let canonical_request = format!(
        "{method}\n{canonical_uri}\n{canonical_query}\n{canonical_headers}\n{signed_headers}\n{payload_hash}"
    );
    let scope = format!("{short_date}/{region}/{service}/aws4_request");
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        hex_lower(&Sha256::digest(canonical_request.as_bytes()))
    );
    let date_key = hmac_sha256(
        format!("AWS4{secret_key}").as_bytes(),
        short_date.as_bytes(),
    )?;
    let region_key = hmac_sha256(&date_key, region.as_bytes())?;
    let service_key = hmac_sha256(&region_key, service.as_bytes())?;
    let signing_key = hmac_sha256(&service_key, b"aws4_request")?;
    let signature = hex_lower(&hmac_sha256(&signing_key, string_to_sign.as_bytes())?);
    Ok(format!(
        "AWS4-HMAC-SHA256 Credential={access_key}/{scope}, SignedHeaders={signed_headers}, Signature={signature}"
    ))
}

fn canonical_query(query: &[(&str, String)]) -> String {
    let mut pairs: Vec<(String, String)> = query
        .iter()
        .map(|(key, value)| {
            (
                percent_encode(key.as_bytes()),
                percent_encode(value.as_bytes()),
            )
        })
        .collect();
    pairs.sort();
    pairs
        .into_iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("&")
}

fn normalize_header(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Result<Vec<u8>, Error> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key)
        .map_err(|_| Error::Other("无法计算上传签名".to_owned()))?;
    mac.update(data);
    Ok(mac.finalize().into_bytes().to_vec())
}

fn cid_for_file<C, P>(
    path: &Path,
    size: u64,
    cancelled: &C,
    progress: &mut P,
) -> Result<String, Error>
where
    C: Fn() -> bool,
    P: FnMut(UploadProgress),
{
    let mut file =
        File::open(path).map_err(|err| Error::Other(format!("无法打开本地文件：{err}")))?;
    let mut hasher = Sha1::new();
    let mut hashed = 0u64;
    if size <= CID_FULL_FILE_LIMIT {
        hash_range(
            &mut file,
            0,
            size,
            &mut hasher,
            cancelled,
            progress,
            &mut hashed,
            size,
        )?;
    } else {
        let total = CID_SAMPLE_BYTES * 3;
        for start in [0, size / 3, size - CID_SAMPLE_BYTES] {
            hash_range(
                &mut file,
                start,
                CID_SAMPLE_BYTES,
                &mut hasher,
                cancelled,
                progress,
                &mut hashed,
                total,
            )?;
        }
    }
    Ok(hex_upper(&hasher.finalize()))
}

fn full_gcid_for_file<C, P>(
    path: &Path,
    size: u64,
    cancelled: &C,
    progress: &mut P,
) -> Result<String, Error>
where
    C: Fn() -> bool,
    P: FnMut(UploadProgress),
{
    ensure_not_cancelled(cancelled)?;
    progress(UploadProgress {
        stage: UploadStage::Hashing,
        bytes: 0,
        total: size,
    });
    let mut file =
        File::open(path).map_err(|err| Error::Other(format!("无法打开本地文件：{err}")))?;
    let block_size = gcid_block_size(size);
    let mut total = Sha1::new();
    let mut read_total = 0u64;
    let mut buf = vec![0u8; 64 * 1024];
    while read_total < size {
        ensure_not_cancelled(cancelled)?;
        let target = block_size.min(size - read_total);
        let mut block = Sha1::new();
        let mut block_read = 0u64;
        while block_read < target {
            ensure_not_cancelled(cancelled)?;
            let want =
                usize::try_from((target - block_read).min(buf.len() as u64)).unwrap_or(buf.len());
            let got = file
                .read(&mut buf[..want])
                .map_err(|err| Error::Other(format!("读取本地文件失败：{err}")))?;
            if got == 0 {
                return Err(Error::Other("文件在上传准备期间被截短".to_owned()));
            }
            block.update(&buf[..got]);
            block_read += got as u64;
            read_total += got as u64;
            progress(UploadProgress {
                stage: UploadStage::Hashing,
                bytes: read_total,
                total: size,
            });
        }
        total.update(block.finalize());
    }
    if read_total != size {
        return Err(Error::Other("文件大小在上传准备期间发生变化".to_owned()));
    }
    Ok(hex_lower(&total.finalize()))
}

#[allow(clippy::too_many_arguments)]
fn hash_range<C, P>(
    file: &mut File,
    start: u64,
    length: u64,
    hasher: &mut Sha1,
    cancelled: &C,
    progress: &mut P,
    hashed: &mut u64,
    total: u64,
) -> Result<(), Error>
where
    C: Fn() -> bool,
    P: FnMut(UploadProgress),
{
    file.seek(SeekFrom::Start(start))
        .map_err(|err| Error::Other(format!("无法定位本地文件：{err}")))?;
    let mut left = length;
    let mut buf = vec![0u8; 64 * 1024];
    while left > 0 {
        ensure_not_cancelled(cancelled)?;
        let want = usize::try_from(left.min(buf.len() as u64)).unwrap_or(buf.len());
        let got = file
            .read(&mut buf[..want])
            .map_err(|err| Error::Other(format!("读取本地文件失败：{err}")))?;
        if got == 0 {
            return Err(Error::Other("文件在上传准备期间被截短".to_owned()));
        }
        hasher.update(&buf[..got]);
        left -= got as u64;
        *hashed += got as u64;
        progress(UploadProgress {
            stage: UploadStage::Hashing,
            bytes: *hashed,
            total,
        });
    }
    Ok(())
}

fn read_exact_range<C>(
    path: &Path,
    offset: u64,
    length: u64,
    cancelled: &C,
) -> Result<Vec<u8>, Error>
where
    C: Fn() -> bool,
{
    ensure_not_cancelled(cancelled)?;
    let mut file =
        File::open(path).map_err(|err| Error::Other(format!("无法打开本地文件：{err}")))?;
    file.seek(SeekFrom::Start(offset))
        .map_err(|err| Error::Other(format!("无法定位本地文件：{err}")))?;
    let capacity =
        usize::try_from(length).map_err(|_| Error::Other("单个上传分片过大".to_owned()))?;
    let mut bytes = vec![0u8; capacity];
    let mut read = 0usize;
    while read < bytes.len() {
        ensure_not_cancelled(cancelled)?;
        let got = file
            .read(&mut bytes[read..])
            .map_err(|err| Error::Other(format!("读取本地文件失败：{err}")))?;
        if got == 0 {
            return Err(Error::Other("文件在上传期间被截短".to_owned()));
        }
        read += got;
    }
    Ok(bytes)
}

fn sha256_hex<C, P>(
    file: &mut File,
    cancelled: &C,
    progress: &mut P,
    total: u64,
) -> Result<Vec<u8>, Error>
where
    C: Fn() -> bool,
    P: FnMut(UploadProgress),
{
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    let mut read = 0u64;
    loop {
        ensure_not_cancelled(cancelled)?;
        let got = file
            .read(&mut buf)
            .map_err(|err| Error::Other(format!("读取本地文件失败：{err}")))?;
        if got == 0 {
            break;
        }
        hasher.update(&buf[..got]);
        read += got as u64;
        progress(UploadProgress {
            stage: UploadStage::Hashing,
            bytes: read,
            total,
        });
    }
    if read != total {
        return Err(Error::Other("文件大小在上传准备期间发生变化".to_owned()));
    }
    Ok(hasher.finalize().to_vec())
}

fn local_file_info(path: &Path) -> Result<LocalFileInfo, Error> {
    let metadata =
        std::fs::metadata(path).map_err(|err| Error::Other(format!("无法读取本地文件：{err}")))?;
    if !metadata.is_file() {
        return Err(Error::InvalidInput("只能上传普通文件".to_owned()));
    }
    let (modified_unix_secs, modified_nanos) = match metadata.modified() {
        Ok(modified) => match modified.duration_since(UNIX_EPOCH) {
            Ok(duration) => (
                i64::try_from(duration.as_secs()).unwrap_or(i64::MAX),
                duration.subsec_nanos(),
            ),
            Err(_) => (0, 0),
        },
        Err(_) => (0, 0),
    };
    Ok(LocalFileInfo {
        size: metadata.len(),
        modified_unix_secs,
        modified_nanos,
    })
}

fn resume_matches(state: &UploadResume, local: &LocalFileInfo, parent_id: &str) -> bool {
    state.version == UPLOAD_RESUME_VERSION
        && state.parent_id == parent_id.trim()
        && state.size == local.size
        && state.local_modified_unix_secs == local.modified_unix_secs
        && state.local_modified_nanos == local.modified_nanos
        && (state.content_uploaded || !credentials_expired(&state.credentials))
}

fn credentials_expired(credentials: &UploadCredentials) -> bool {
    let expiration = credentials.expiration.trim();
    if expiration.is_empty() {
        return false;
    }
    match chrono::DateTime::parse_from_rfc3339(expiration) {
        Ok(expiration) => expiration.with_timezone(&Utc) <= Utc::now(),
        // 官方格式变化时不拿“看起来过期”当事实，交给真正的 S3 请求判断。
        Err(_) => false,
    }
}

fn part_length(part_number: u64, part_size: u64, size: u64) -> u64 {
    if part_number == 0 || part_size == 0 {
        return 0;
    }
    let offset = (part_number - 1).saturating_mul(part_size);
    size.saturating_sub(offset).min(part_size)
}

fn completed_part_bytes(parts: &BTreeMap<u64, UploadPart>, part_size: u64, size: u64) -> u64 {
    parts
        .keys()
        .map(|number| part_length(*number, part_size, size))
        .sum()
}

fn valid_completed_part(part: &UploadPart, part_count: u64, part_size: u64, size: u64) -> bool {
    part.number > 0
        && part.number <= part_count
        && !part.etag.trim().is_empty()
        && part.size == part_length(part.number, part_size, size)
}

fn multipart_part_size(size: u64) -> u64 {
    let mut part = SMALL_FILE_LIMIT.max(1);
    while size.div_ceil(part) > MAX_MULTIPART_PARTS {
        part = part.saturating_mul(2);
    }
    part
}

fn gcid_block_size(size: u64) -> u64 {
    let mut block: u64 = 0x40000;
    while size > block.saturating_mul(0x200) && block < 0x200000 {
        block <<= 1;
    }
    block
}

fn validate_name(name: &str) -> Result<(), Error> {
    if name.is_empty() {
        return Err(Error::InvalidInput("文件名不能为空".to_owned()));
    }
    if name.contains('/') {
        return Err(Error::InvalidInput("文件名里不能包含 /".to_owned()));
    }
    if name.chars().any(char::is_control) {
        return Err(Error::InvalidInput("文件名里不能包含控制字符".to_owned()));
    }
    if name.len() > MAX_NAME_BYTES {
        return Err(Error::InvalidInput(format!(
            "文件名太长（最多 {MAX_NAME_BYTES} 字节）"
        )));
    }
    Ok(())
}

fn ensure_file_size(path: &Path, expected: u64) -> Result<(), Error> {
    let actual = std::fs::metadata(path)
        .map_err(|err| Error::Other(format!("无法读取本地文件：{err}")))?
        .len();
    if actual != expected {
        return Err(Error::Other(
            "文件大小在上传期间发生变化，请重新上传".to_owned(),
        ));
    }
    Ok(())
}

fn is_hard_auth_error(error: &Error) -> bool {
    matches!(
        error,
        Error::Auth { .. } | Error::Captcha { .. } | Error::RateLimited { .. }
    )
}

fn is_retryable_api_error(error: &Error) -> bool {
    matches!(error, Error::Network(_))
        || matches!(
            error,
            Error::Api { code, .. } if matches!(*code, 408 | 425 | 500 | 502 | 503 | 504)
        )
}

fn keep_resume_for_api_error(error: &Error) -> bool {
    matches!(
        error,
        Error::Network(_) | Error::RateLimited { .. } | Error::Auth { .. } | Error::Captcha { .. }
    ) || matches!(
        error,
        Error::Api { code, .. } if matches!(*code, 408 | 425 | 500 | 502 | 503 | 504)
    )
}

fn preserve_after_error(error: Error) -> UploadError {
    if is_hard_auth_error(&error) {
        UploadError::hard(error)
    } else {
        UploadError::retryable(error)
    }
}

fn ensure_not_cancelled<C>(cancelled: &C) -> Result<(), Error>
where
    C: Fn() -> bool,
{
    if cancelled() {
        Err(Error::Other("上传已取消".to_owned()))
    } else {
        Ok(())
    }
}

fn sleep_cancellable<C>(duration: Duration, cancelled: &C) -> Result<(), Error>
where
    C: Fn() -> bool,
{
    let started = Instant::now();
    while started.elapsed() < duration {
        ensure_not_cancelled(cancelled)?;
        thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

fn ensure_s3_success(status: StatusCode, body: &str) -> Result<(), Error> {
    if status.is_success() {
        return Ok(());
    }
    Err(s3_error(status, body))
}

fn s3_error(status: StatusCode, body: &str) -> Error {
    if let Some((code, message)) = parse_s3_error_body(body) {
        if matches!(code.as_str(), "QuotaExceeded" | "SlowDown")
            && status != StatusCode::TOO_MANY_REQUESTS
        {
            return Error::RateLimited {
                reason: format_s3_error(&code, &message),
            };
        }
        return Error::Other(format_s3_error(&code, &message));
    }
    if status == StatusCode::TOO_MANY_REQUESTS {
        return Error::RateLimited {
            reason: "上传服务返回 HTTP 429".to_owned(),
        };
    }
    Error::Other(format!("上传服务返回 HTTP {status}"))
}

fn format_s3_error(code: &str, message: &str) -> String {
    if message.is_empty() {
        format!("上传服务返回错误：{code}")
    } else {
        format!("上传服务返回错误：{code} · {message}")
    }
}

fn parse_s3_error_body(body: &str) -> Option<(String, String)> {
    let code = xml_text(body.as_bytes(), b"Code")?;
    let message = xml_text(body.as_bytes(), b"Message").unwrap_or_default();
    Some((code, message))
}

fn parse_list_parts(xml: &str) -> Result<ListPartsPage, Error> {
    let mut reader = Reader::from_str(xml);
    let mut parts = Vec::new();
    let mut current: Option<UploadPart> = None;
    let mut field: Vec<u8> = Vec::new();
    let mut text = String::new();
    let mut truncated = false;
    let mut next_marker = None;
    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) => {
                let name = event.local_name().as_ref().to_vec();
                if name == b"Part" {
                    current = Some(UploadPart {
                        number: 0,
                        etag: String::new(),
                        size: 0,
                    });
                    field.clear();
                    continue;
                }
                if matches!(
                    name.as_slice(),
                    b"PartNumber" | b"ETag" | b"Size" | b"IsTruncated" | b"NextPartNumberMarker"
                ) {
                    field = name;
                    text.clear();
                }
            }
            Ok(Event::Text(event)) if !field.is_empty() => {
                text.push_str(&String::from_utf8_lossy(event.as_ref()));
            }
            Ok(Event::GeneralRef(event)) if !field.is_empty() => {
                push_xml_ref(&mut text, event.as_ref());
            }
            Ok(Event::End(event)) => {
                let name = event.local_name().as_ref().to_vec();
                if name == b"Part" {
                    let Some(part) = current.take() else {
                        return Err(Error::ApiChanged("分片列表缺少分片内容".to_owned()));
                    };
                    if part.number == 0 || part.etag.trim().is_empty() || part.size == 0 {
                        return Err(Error::ApiChanged("分片列表包含不完整分片".to_owned()));
                    }
                    parts.push(part);
                    field.clear();
                    text.clear();
                    continue;
                }
                if name != field {
                    continue;
                }
                match name.as_slice() {
                    b"PartNumber" if current.is_some() => {
                        let number = parse_xml_u64(&text, "分片号")?;
                        if let Some(part) = current.as_mut() {
                            part.number = number;
                        }
                    }
                    b"ETag" if current.is_some() => {
                        if let Some(part) = current.as_mut() {
                            part.etag = text.trim().to_owned();
                        }
                    }
                    b"Size" if current.is_some() => {
                        let size = parse_xml_u64(&text, "分片大小")?;
                        if let Some(part) = current.as_mut() {
                            part.size = size;
                        }
                    }
                    b"IsTruncated" => {
                        truncated = matches!(text.trim(), "true" | "1");
                    }
                    b"NextPartNumberMarker" => {
                        next_marker = Some(parse_xml_u64(&text, "下一分片标记")?);
                    }
                    _ => {}
                }
                field.clear();
                text.clear();
            }
            Ok(Event::Eof) => break,
            Err(error) => {
                return Err(Error::ApiChanged(format!("无法解析分片列表：{error}")));
            }
            _ => {}
        }
    }
    Ok(ListPartsPage {
        parts,
        truncated,
        next_marker,
    })
}

fn parse_xml_u64(text: &str, what: &str) -> Result<u64, Error> {
    text.trim()
        .parse::<u64>()
        .map_err(|_| Error::ApiChanged(format!("分片列表的{what}无效")))
}

fn push_xml_ref(out: &mut String, reference: &[u8]) {
    match reference {
        b"amp" => out.push('&'),
        b"lt" => out.push('<'),
        b"gt" => out.push('>'),
        b"quot" => out.push('"'),
        b"apos" => out.push('\''),
        _ => {
            out.push('&');
            out.push_str(&String::from_utf8_lossy(reference));
            out.push(';');
        }
    }
}

fn xml_text(xml: &[u8], tag: &[u8]) -> Option<String> {
    let mut reader = Reader::from_reader(xml);
    let mut in_wanted = false;
    let mut out = String::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) => {
                in_wanted = event.name().as_ref() == tag;
                if in_wanted {
                    out.clear();
                }
            }
            Ok(Event::Text(event)) if in_wanted => {
                out.push_str(&String::from_utf8_lossy(event.as_ref()));
            }
            Ok(Event::GeneralRef(event)) if in_wanted => {
                let reference = event.as_ref();
                match reference {
                    b"amp" => out.push('&'),
                    b"lt" => out.push('<'),
                    b"gt" => out.push('>'),
                    b"quot" => out.push('"'),
                    b"apos" => out.push('\''),
                    _ => {
                        let name = String::from_utf8_lossy(reference);
                        out.push('&');
                        out.push_str(&name);
                        out.push(';');
                    }
                }
            }
            Ok(Event::End(event)) if event.name().as_ref() == tag && in_wanted => {
                return Some(out);
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    None
}

/// 上传请求体读取器：S3 还在读取本地数据时也能看到取消信号。
///
/// `reqwest::blocking::Body::sized` 要求读取器 `'static`，所以取消判据本身
/// 必须是可克隆、线程安全且拥有所有权的闭包。
struct CancelReader<R, C> {
    inner: R,
    cancelled: C,
}

impl<R, C> CancelReader<R, C>
where
    C: Fn() -> bool,
{
    fn new(inner: R, cancelled: C) -> Self {
        Self { inner, cancelled }
    }
}

impl<R, C> Read for CancelReader<R, C>
where
    R: Read,
    C: Fn() -> bool,
{
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if (self.cancelled)() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "upload cancelled",
            ));
        }
        self.inner.read(buf)
    }
}

/// 在取消读取器外面再扣全局限速令牌。
struct RateLimitedReader<R, C> {
    inner: R,
    limiter: Option<Arc<UploadRateLimiter>>,
    cancelled: C,
}

impl<R, C> RateLimitedReader<R, C>
where
    C: Fn() -> bool,
{
    fn new(inner: R, limiter: Option<Arc<UploadRateLimiter>>, cancelled: C) -> Self {
        Self {
            inner,
            limiter,
            cancelled,
        }
    }
}

impl<R, C> Read for RateLimitedReader<R, C>
where
    R: Read,
    C: Fn() -> bool,
{
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buf)?;
        if read > 0 {
            if let Some(limiter) = self.limiter.as_ref() {
                if !limiter.wait(read, &self.cancelled) {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::Interrupted,
                        "upload cancelled",
                    ));
                }
            }
        }
        Ok(read)
    }
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn read_limited(mut response: reqwest::blocking::Response, limit: u64) -> Result<String, Error> {
    let mut bytes = Vec::new();
    response
        .by_ref()
        .take(limit)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::Network("读取上传服务响应失败".to_owned()))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn s3_network_error(error: reqwest::Error) -> Error {
    if error.is_timeout() {
        Error::Network("上传请求超时".to_owned())
    } else if error.is_connect() {
        Error::Network("无法连接上传服务器".to_owned())
    } else {
        Error::Network("上传连接中断".to_owned())
    }
}

fn non_empty(value: String) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn non_empty_name(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

fn encode_path(key: &str) -> String {
    let encoded = percent_encode_keep_slash(key.as_bytes());
    if encoded.starts_with('/') {
        encoded
    } else {
        format!("/{encoded}")
    }
}

fn percent_encode_keep_slash(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for byte in bytes {
        if *byte == b'/' || is_unreserved(*byte) {
            out.push(char::from(*byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn percent_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for byte in bytes {
        if is_unreserved(*byte) {
            out.push(char::from(*byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn is_unreserved(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~')
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn hex_upper(bytes: &[u8]) -> String {
    hex_lower(bytes).to_ascii_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::upload_state::valid_bucket;
    use std::io::Cursor;

    #[test]
    fn zero_byte_cid_is_the_sha1_of_empty_input() {
        let path = temp_path("zero");
        std::fs::write(&path, []).expect("create empty file");
        let mut progress = |_| {};
        let cid = cid_for_file(&path, 0, &|| false, &mut progress).expect("cid");
        let _ = std::fs::remove_file(path);
        assert_eq!(cid, "DA39A3EE5E6B4B0D3255BFEF95601890AFD80709");
    }

    #[test]
    fn small_file_cid_hashes_the_whole_file() {
        let path = temp_path("small");
        std::fs::write(&path, b"hello").expect("create file");
        let mut progress = |_| {};
        let cid = cid_for_file(&path, 5, &|| false, &mut progress).expect("cid");
        let _ = std::fs::remove_file(path);
        assert_eq!(cid, "AAF4C61DDCC5E8A2DABEDE0F3B482CD9AEA9434D");
    }

    #[test]
    fn gcid_block_size_matches_rclone_boundaries() {
        assert_eq!(gcid_block_size(0x40000 * 0x200), 0x40000);
        assert_eq!(gcid_block_size(0x40000 * 0x200 + 1), 0x80000);
        assert_eq!(gcid_block_size(u64::MAX), 0x200000);
    }

    #[test]
    fn multipart_part_size_respects_the_ten_thousand_part_cap() {
        let part = multipart_part_size(5 * 1024 * 1024 * 10_001);
        assert!((5_u64 * 1024 * 1024 * 10_001).div_ceil(part) <= MAX_MULTIPART_PARTS);
        assert!(part >= SMALL_FILE_LIMIT);
    }

    #[test]
    fn path_encoding_keeps_separators_but_encodes_spaces_and_unicode() {
        assert_eq!(encode_path("a b/中文.mkv"), "/a%20b/%E4%B8%AD%E6%96%87.mkv");
    }

    #[test]
    fn canonical_query_is_sorted_and_encoded() {
        let query = vec![
            ("uploadId", "a b+c".to_owned()),
            ("partNumber", "2".to_owned()),
        ];
        assert_eq!(canonical_query(&query), "partNumber=2&uploadId=a%20b%2Bc");
    }

    #[test]
    fn header_values_collapse_whitespace() {
        assert_eq!(normalize_header(" a \t b\n c "), "a b c");
    }

    #[test]
    fn sigv4_matches_an_independent_aws_v4_calculation() {
        let amz_date = format!("{}{}", "202601", "01T000000Z");
        let query = vec![
            ("uploadId", "abc+/=".to_owned()),
            ("partNumber", "1".to_owned()),
        ];
        let authorization = sign_request(
            "PUT",
            "/uploads/a%20b",
            &query,
            "bucket.mypikpak.com",
            Some(5),
            Some("application/octet-stream"),
            "token",
            "abc123",
            &amz_date,
            "20260101",
            "pikpak",
            "s3",
            "access",
            "secret",
        )
        .expect("signature");
        let credential = [
            "Cred",
            "ential=",
            "access",
            "/20260101/pikpak/s3/aws4_request",
        ]
        .concat();
        let expected = [
            "AWS4-HMAC-SHA256 ",
            credential.as_str(),
            ", SignedHeaders=content-length;content-type;host;x-amz-content-sha256;",
            "x-amz-date;x-amz-security-token, ",
            "Signature=85736f30a4e1086c370356f75c60edda5f87cb60aabc37a868ce3544cd677608",
        ]
        .concat();
        assert_eq!(authorization, expected);
    }

    #[test]
    fn cancel_reader_stops_before_reading_more_bytes() {
        let mut reader = CancelReader::new(Cursor::new(b"payload".to_vec()), || true);
        let mut buffer = [0u8; 4];
        let error = reader.read(&mut buffer).expect_err("must be cancelled");
        assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
    }

    #[test]
    fn small_payload_hash_rejects_a_file_that_changed_size() {
        let path = temp_path("changed-size");
        std::fs::write(&path, b"hello").expect("create file");
        let mut file = File::open(&path).expect("open file");
        let mut progress = |_| {};
        assert!(sha256_hex(&mut file, &|| false, &mut progress, 4).is_err());
        file.seek(SeekFrom::Start(0)).expect("rewind file");
        assert!(sha256_hex(&mut file, &|| false, &mut progress, 6).is_err());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn upload_rechecks_the_original_file_size() {
        let path = temp_path("size-check");
        std::fs::write(&path, b"hello").expect("create file");
        assert!(ensure_file_size(&path, 5).is_ok());
        assert!(ensure_file_size(&path, 4).is_err());
        assert!(ensure_file_size(&path, 6).is_err());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn resume_identity_accepts_a_collision_renamed_ticket() {
        let local = LocalFileInfo {
            size: 12,
            modified_unix_secs: 34,
            modified_nanos: 56,
        };
        let mut state = UploadResume::new(
            local.size,
            local.modified_unix_secs,
            local.modified_nanos,
            "parent".into(),
            "file".into(),
            "task".into(),
            "GCID".into(),
            "电影 (1).mkv".into(),
            "电影 (1).mkv".into(),
            credentials("2099-01-01T00:00:00Z"),
        );
        assert!(
            resume_matches(&state, &local, "parent"),
            "撞名后的票据名不能把断点误判为失效"
        );

        let changed = LocalFileInfo {
            size: 12,
            modified_unix_secs: 34,
            modified_nanos: 56,
        };
        let changed = LocalFileInfo {
            size: changed.size + 1,
            ..changed
        };
        assert!(!resume_matches(&state, &changed, "parent"));
        assert!(!resume_matches(&state, &local, "other-parent"));

        state.credentials = credentials("2000-01-01T00:00:00Z");
        assert!(!resume_matches(&state, &local, "parent"));
        state.content_uploaded = true;
        assert!(
            resume_matches(&state, &local, "parent"),
            "内容已提交时只需继续任务，不再依赖过期的 S3 凭据"
        );
    }

    #[test]
    fn non_retrying_s3_probes_keep_hard_limits_hard() {
        let denied = "<Error><Code>AccessDenied</Code><Message>no</Message></Error>";
        let forbidden = upload_error_from_s3(StatusCode::FORBIDDEN, denied);
        assert!(forbidden.keep_resume);
        assert!(forbidden.stop_queue);

        let rate_limited = upload_error_from_s3(StatusCode::TOO_MANY_REQUESTS, "");
        assert!(rate_limited.keep_resume);
        assert!(rate_limited.stop_queue);

        let quota = "<Error><Code>QuotaExceeded</Code><Message>full</Message></Error>";
        let quota = upload_error_from_s3(StatusCode::BAD_REQUEST, quota);
        assert!(quota.keep_resume);
        assert!(quota.stop_queue);

        let server_error = upload_error_from_s3(StatusCode::INTERNAL_SERVER_ERROR, "");
        assert!(!server_error.keep_resume);
        assert!(!server_error.stop_queue);
    }

    fn credentials(expiration: &str) -> UploadCredentials {
        UploadCredentials {
            access_key_id: "access".into(),
            access_key_secret: "secret".into(),
            security_token: "token".into(),
            bucket: "bucket".into(),
            key: "key".into(),
            endpoint: "https://bucket.mypikpak.com".into(),
            expiration: expiration.into(),
        }
    }

    #[test]
    fn empty_sha256_constant_is_stable() {
        assert_eq!(
            hex_lower(&Sha256::digest([])),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn s3_error_xml_is_sanitized_to_code_and_message() {
        let xml = "<Error><Code>AccessDenied</Code><Message>bad &amp; denied</Message></Error>";
        assert_eq!(
            parse_s3_error_body(xml),
            Some(("AccessDenied".to_owned(), "bad & denied".to_owned()))
        );
    }

    #[test]
    fn transient_s3_failures_are_retryable_but_client_errors_are_not() {
        assert!(matches!(
            s3_attempt_error(StatusCode::SERVICE_UNAVAILABLE, ""),
            UploadAttemptError::Retryable(_)
        ));
        let timeout = "<Error><Code>RequestTimeout</Code><Message>try again</Message></Error>";
        assert!(matches!(
            s3_attempt_error(StatusCode::BAD_REQUEST, timeout),
            UploadAttemptError::Retryable(_)
        ));
        let denied = "<Error><Code>AccessDenied</Code><Message>no</Message></Error>";
        assert!(matches!(
            s3_attempt_error(StatusCode::FORBIDDEN, denied),
            UploadAttemptError::Hard(_)
        ));
        assert!(matches!(
            s3_attempt_error(StatusCode::TOO_MANY_REQUESTS, ""),
            UploadAttemptError::Hard(_)
        ));
        let quota = "<Error><Code>QuotaExceeded</Code><Message>full</Message></Error>";
        assert!(matches!(
            s3_attempt_error(StatusCode::BAD_REQUEST, quota),
            UploadAttemptError::Hard(_)
        ));
    }

    #[test]
    fn upload_retry_backoff_is_bounded() {
        assert_eq!(retry_delay(0), UPLOAD_RETRY_BASE);
        assert_eq!(retry_delay(1), UPLOAD_RETRY_BASE.saturating_mul(2));
        assert_eq!(retry_delay(20), UPLOAD_RETRY_MAX);
    }

    #[test]
    fn shared_part_bytes_can_be_replayed() {
        let bytes = Arc::new(b"reusable".to_vec());
        let mut first = CancelReader::new(ByteSliceReader::new(Arc::clone(&bytes)), || false);
        let mut second = CancelReader::new(ByteSliceReader::new(bytes), || false);
        let mut first_out = Vec::new();
        let mut second_out = Vec::new();
        first.read_to_end(&mut first_out).expect("read first body");
        second
            .read_to_end(&mut second_out)
            .expect("read replayed body");
        assert_eq!(first_out, b"reusable");
        assert_eq!(second_out, b"reusable");
    }

    #[test]
    fn upload_names_are_validated_before_network_work() {
        assert!(validate_name("电影.mkv").is_ok());
        assert!(validate_name(" 两侧空格保留.mkv ").is_ok());
        assert!(validate_name("").is_err());
        assert!(validate_name("a/b").is_err());
        assert!(validate_name("a\nb").is_err());
        assert_eq!(
            non_empty_name(" 两侧空格保留.mkv ".to_owned()).as_deref(),
            Some(" 两侧空格保留.mkv ")
        );
    }

    #[test]
    fn bucket_validation_rejects_host_injection() {
        assert!(valid_bucket("abc-123"));
        assert!(!valid_bucket("abc.example"));
        assert!(!valid_bucket("../abc"));
        assert!(!valid_bucket("-abc"));
    }

    fn temp_path(tag: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "kichi-upload-{tag}-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        path
    }
}
