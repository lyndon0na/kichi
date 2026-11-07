//! S3 错误分类与可重放操作的退避重试。

use std::time::Duration;

use reqwest::StatusCode;

use crate::upload_state::UploadError;
use crate::Error;

use super::s3_xml::parse_s3_error_body;
use super::util::{ensure_not_cancelled, is_hard_auth_error, sleep_cancellable};
use super::{UPLOAD_MAX_ATTEMPTS, UPLOAD_RETRY_BASE, UPLOAD_RETRY_MAX};

pub(super) enum UploadAttemptError {
    Retryable(Error),
    Fatal(Error),
    /// 不自动重试，但保留断点并停住本轮上传队列。
    Hard(Error),
    /// 探测请求已经带回了完整的上传错误分类，直接原样上抛。
    Preserve(UploadError),
}

pub(super) enum ListPartsError {
    NoSuchUpload,
    Other(UploadError),
}

/// 重放可安全重复的 S3 操作。票据申请和 CreateMultipartUpload 不经过这里，
/// 避免响应丢失时在云端留下两个请求各自创建的上传记录。
pub(super) fn retry_upload<T, C, F>(cancelled: &C, mut operation: F) -> Result<T, UploadError>
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

pub(super) fn retry_delay(attempt: u32) -> Duration {
    let factor = 1u32 << attempt.min(4);
    UPLOAD_RETRY_BASE
        .saturating_mul(factor)
        .min(UPLOAD_RETRY_MAX)
}

pub(super) fn s3_attempt_error(status: StatusCode, body: &str) -> UploadAttemptError {
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

pub(super) fn upload_error_from_s3(status: StatusCode, body: &str) -> UploadError {
    let parsed = parse_s3_error_body(body);
    let error = s3_error(status, body);
    if is_hard_s3_response(status, parsed.as_ref().map(|(code, _)| code.as_str())) {
        UploadError::hard(error)
    } else {
        UploadError::fatal(error)
    }
}

pub(super) fn is_hard_s3_response(status: StatusCode, code: Option<&str>) -> bool {
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

pub(super) fn retryable_status(status: StatusCode) -> bool {
    matches!(status.as_u16(), 408 | 425 | 500 | 502 | 503 | 504)
}

pub(super) fn retryable_s3_code(code: &str) -> bool {
    matches!(
        code,
        "RequestTimeout" | "InternalError" | "ServiceUnavailable" | "SlowDown"
    )
}

pub(super) fn ensure_s3_success(status: StatusCode, body: &str) -> Result<(), Error> {
    if status.is_success() {
        return Ok(());
    }
    Err(s3_error(status, body))
}

pub(super) fn s3_error(status: StatusCode, body: &str) -> Error {
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

pub(super) fn format_s3_error(code: &str, message: &str) -> String {
    if message.is_empty() {
        format!("上传服务返回错误：{code}")
    } else {
        format!("上传服务返回错误：{code} · {message}")
    }
}
