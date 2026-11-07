//! S3 对象操作：单段 PUT、分片上传、ListParts、Complete / Abort。

use std::fs::File;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use reqwest::blocking::{Body, Client};
use reqwest::{Method, StatusCode, Url};
use sha2::{Digest as Sha256Digest, Sha256};

use crate::upload_progress::{ProgressReader, UploadByteTracker};
use crate::upload_rate::UploadRateLimiter;
use crate::upload_state::{UploadCredentials, UploadError, UploadPart};
use crate::{Config, Error};

use super::io::{
    encode_path, hex_lower, ByteSliceReader, CancelReader, FileRangeReader, RateLimitedReader,
};
use super::s3_error::{
    ensure_s3_success, retry_upload, s3_attempt_error, upload_error_from_s3, ListPartsError,
    UploadAttemptError,
};
use super::s3_request::S3RequestBody;
use super::s3_xml::{parse_list_parts, parse_s3_error_body, xml_escape, xml_text};
use super::util::{ensure_file_size, ensure_not_cancelled, preserve_after_error};
use super::MAX_MULTIPART_PARTS;

pub(super) struct S3Target {
    pub(super) client: Arc<Client>,
    pub(super) credentials: UploadCredentials,
    pub(super) host: String,
    pub(super) path: String,
    pub(super) limiter: Option<Arc<UploadRateLimiter>>,
}

impl S3Target {
    pub(super) fn new_with_client(
        config: &Config,
        credentials: &UploadCredentials,
        limiter: Option<Arc<UploadRateLimiter>>,
        client: Arc<Client>,
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
        Ok(Self {
            client,
            credentials: credentials.clone(),
            host,
            path: encode_path(credentials.key.trim()),
            limiter,
        })
    }

    pub(super) fn new_with_timeout(
        config: &Config,
        credentials: &UploadCredentials,
        limiter: Option<Arc<UploadRateLimiter>>,
        timeout: Duration,
    ) -> Result<Self, Error> {
        let client = Arc::new(
            Client::builder()
                .user_agent(&config.user_agent)
                .connect_timeout(Duration::from_secs(15))
                // 单次 S3 请求最坏 5 分钟；不是整文件的固定总分片时限。
                .timeout(timeout)
                // 上传请求只允许直连收窄后的 mypikpak.com 主机，不能跟去别处。
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| Error::Network("无法创建上传 HTTP 客户端".to_owned()))?,
        );
        Self::new_with_client(config, credentials, limiter, client)
    }

    pub(super) fn put_object<C>(
        &self,
        path: &Path,
        size: u64,
        payload_hash: &str,
        content_type: &str,
        cancelled: &C,
        progress: Option<&UploadByteTracker>,
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
            let counter = progress.map(|tracker| tracker.register(0));
            let body = match counter {
                Some(counter) => Body::sized(ProgressReader::new(body, counter), size),
                None => Body::sized(body, size),
            };
            let response = self
                .send(
                    Method::PUT,
                    &[],
                    payload_hash,
                    S3RequestBody {
                        content_length: (size > 0).then_some(size),
                        content_type: Some(content_type),
                        body: Some(body),
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

    pub(super) fn create_multipart<C>(&self, cancelled: &C) -> Result<String, UploadError>
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

    pub(super) fn list_parts<C>(
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

    #[allow(clippy::too_many_arguments)]
    pub(super) fn upload_part_from_file<C>(
        &self,
        upload_id: &str,
        part_number: u64,
        path: &Path,
        offset: u64,
        length: u64,
        file_size: u64,
        payload_hash: &str,
        cancelled: &C,
        progress: Option<&UploadByteTracker>,
    ) -> Result<String, UploadError>
    where
        C: Fn() -> bool + Clone + Send + Sync + 'static,
    {
        retry_upload(cancelled, |_attempt| {
            ensure_file_size(path, file_size).map_err(UploadAttemptError::Fatal)?;
            let reader =
                FileRangeReader::open(path, offset, length).map_err(UploadAttemptError::Fatal)?;
            let body = RateLimitedReader::new(
                CancelReader::new(reader, cancelled.clone()),
                self.limiter.clone(),
                cancelled.clone(),
            );
            let counter = progress.map(|tracker| tracker.register(part_number));
            let body = match counter {
                Some(counter) => Body::sized(ProgressReader::new(body, counter), length),
                None => Body::sized(body, length),
            };
            let response = self
                .send(
                    Method::PUT,
                    &[
                        ("partNumber", part_number.to_string()),
                        ("uploadId", upload_id.to_owned()),
                    ],
                    payload_hash,
                    S3RequestBody {
                        content_length: Some(length),
                        content_type: Some("application/octet-stream"),
                        body: Some(body),
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

    pub(super) fn complete_multipart<C>(
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

    pub(super) fn object_exists<C>(&self, size: u64, cancelled: &C) -> Result<bool, UploadError>
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

    pub(super) fn abort_multipart(&self, upload_id: &str) -> Result<(), Error> {
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
}
