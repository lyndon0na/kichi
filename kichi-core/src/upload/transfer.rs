//! GCID 计算与单段 / 分片传输编排。

use std::collections::BTreeMap;
use std::fs::File;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;

use serde::Deserialize;

use crate::drive::DriveClient;
use crate::upload_progress::{UploadByteTracker, LIVE_INTERVAL};
use crate::upload_rate::UploadRateLimiter;
use crate::upload_state::{UploadError, UploadPart, UploadResume};
use crate::Error;

use super::cid::{cid_for_file, full_gcid_for_file, sha256_file_range, sha256_hex};
use super::io::hex_lower;
use super::s3::S3Target;
use super::s3_error::ListPartsError;
use super::util::{
    completed_part_bytes, ensure_file_size, ensure_not_cancelled, is_hard_auth_error,
    multipart_part_size, part_length, valid_completed_part,
};
use super::{
    UploadProgress, UploadStage, CID_FULL_FILE_LIMIT, CID_SAMPLE_BYTES, MAX_MULTIPART_PARTS,
    MAX_PARALLEL_PARTS, SMALL_FILE_LIMIT,
};

#[derive(Debug, Deserialize, Default)]
struct CidResponse {
    #[serde(default)]
    gcid: String,
}

impl DriveClient {
    pub(super) fn gcid_for_file<C, P>(
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
            completed_bytes: None,
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
    pub(super) fn transfer_to_s3<C, P, F>(
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
        let target = S3Target::new_with_client(
            self.config(),
            &state.credentials,
            limiter.cloned(),
            self.upload_client()?,
        )?;
        if size <= SMALL_FILE_LIMIT {
            if state.content_uploaded {
                return Ok((state.bytes_sent, 0));
            }
            let mut file =
                File::open(path).map_err(|err| Error::Other(format!("无法打开本地文件：{err}")))?;
            progress(UploadProgress {
                stage: UploadStage::Hashing,
                bytes: 0,
                completed_bytes: None,
                total: size,
            });
            let digest = sha256_hex(&mut file, cancelled, progress, size)?;
            let payload_hash = hex_lower(&digest);
            progress(UploadProgress {
                stage: UploadStage::Uploading,
                bytes: 0,
                completed_bytes: Some(0),
                total: size,
            });
            drop(file);
            let tracker = UploadByteTracker::new(0);
            let uploaded = thread::scope(|scope| {
                let handle = scope.spawn(|| {
                    target.put_object(
                        path,
                        size,
                        &payload_hash,
                        "application/octet-stream",
                        cancelled,
                        Some(&tracker),
                    )
                });
                // 单次 PUT 由 reqwest 在后台线程阻塞读取；主线程按固定间隔
                // 汇总在途字节，不能等整次请求结束才更新界面。
                loop {
                    if handle.is_finished() {
                        break;
                    }
                    thread::sleep(LIVE_INTERVAL);
                    if handle.is_finished() {
                        break;
                    }
                    progress(tracker.progress(size));
                }
                match handle.join() {
                    Ok(result) => result,
                    Err(_) => Err(UploadError::fatal(Error::Other(
                        "上传线程异常中断".to_owned(),
                    ))),
                }
            });
            uploaded.map_err(|failure| UploadError {
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
                completed_bytes: Some(size),
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
            completed_bytes: Some(state.bytes_sent),
            total: size,
        });

        let pending_parts: Vec<u64> = (1..=part_count)
            .filter(|part_number| !completed.contains_key(part_number))
            .collect();
        let mut sent_this_run = 0u64;
        if !pending_parts.is_empty() {
            let tracker = UploadByteTracker::new(state.bytes_sent);
            let upload_id = state.upload_id.clone();
            let stop = AtomicBool::new(false);
            let cursor = AtomicUsize::new(0);
            let first_failure = std::sync::Mutex::new(None);
            let (sender, receiver) = mpsc::channel();
            let workers = pending_parts.len().min(MAX_PARALLEL_PARTS);

            std::thread::scope(|scope| {
                for worker in 0..workers {
                    let sender = sender.clone();
                    let pending_parts = &pending_parts;
                    let stop = &stop;
                    let cursor = &cursor;
                    let first_failure = &first_failure;
                    let target = &target;
                    let upload_id = &upload_id;
                    let cancelled = cancelled.clone();
                    let tracker = &tracker;
                    let spawned = std::thread::Builder::new()
                        .name(format!("kichi-upload-part-{worker}"))
                        .spawn_scoped(scope, move || {
                            let worker = || loop {
                                if stop.load(Ordering::SeqCst) || cancelled() {
                                    break;
                                }
                                let position = cursor.fetch_add(1, Ordering::SeqCst);
                                let Some(part_number) = pending_parts.get(position).copied() else {
                                    break;
                                };
                                let offset = (part_number - 1) * part_size;
                                let length = part_length(part_number, part_size, size);
                                let result = (|| {
                                    ensure_not_cancelled(&cancelled)?;
                                    ensure_file_size(path, size)?;
                                    let digest =
                                        sha256_file_range(path, offset, length, &cancelled)?;
                                    target.upload_part_from_file(
                                        upload_id,
                                        part_number,
                                        path,
                                        offset,
                                        length,
                                        size,
                                        &hex_lower(&digest),
                                        &cancelled,
                                        Some(tracker),
                                    )
                                })();
                                match result {
                                    Ok(etag) => {
                                        if sender.send((part_number, length, etag)).is_err() {
                                            break;
                                        }
                                    }
                                    Err(error) => {
                                        // 另一条分片线程因本地 stop 收尾时也会返回
                                        // “已取消”；只记录真正触发停止的那条错误，
                                        // 避免把硬错误覆盖成普通取消。
                                        if !cancelled() && !stop.swap(true, Ordering::SeqCst) {
                                            let mut slot = first_failure
                                                .lock()
                                                .unwrap_or_else(|poisoned| poisoned.into_inner());
                                            if slot.is_none() {
                                                *slot = Some(error);
                                            }
                                        }
                                        break;
                                    }
                                }
                            };
                            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(worker))
                                .is_err()
                            {
                                stop.store(true, Ordering::SeqCst);
                                let mut slot = first_failure
                                    .lock()
                                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                                if slot.is_none() {
                                    *slot = Some(UploadError::fatal(Error::Other(
                                        "上传分片线程异常中断".to_owned(),
                                    )));
                                }
                            }
                        });
                    if let Err(error) = spawned {
                        let mut slot = first_failure
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        if slot.is_none() {
                            *slot = Some(UploadError::fatal(Error::Other(format!(
                                "无法创建上传分片线程：{error}"
                            ))));
                        }
                        stop.store(true, Ordering::SeqCst);
                        break;
                    }
                }
                drop(sender);
                loop {
                    match receiver.recv_timeout(LIVE_INTERVAL) {
                        Ok((part_number, length, etag)) => {
                            // 先撤掉在途计数，再把整片并入已完成，避免双计。
                            tracker.unregister(part_number);
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
                            tracker.set_completed(state.bytes_sent);
                            state.touch();
                            progress(tracker.progress(size));
                            checkpoint(state);
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            progress(tracker.progress(size));
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
            });

            if let Some(error) = first_failure
                .into_inner()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
            {
                return Err(error);
            }
        }
        ensure_not_cancelled(cancelled)?;
        if state.parts.len() as u64 != part_count {
            return Err(UploadError::retryable(Error::Other(
                "上传分片没有全部完成，请稍后重试".to_owned(),
            )));
        }
        ensure_file_size(path, size)?;
        target.complete_multipart(&state.upload_id, &state.parts, cancelled)?;
        state.content_uploaded = true;
        state.touch();
        checkpoint(state);
        Ok((state.bytes_sent, sent_this_run))
    }
}
