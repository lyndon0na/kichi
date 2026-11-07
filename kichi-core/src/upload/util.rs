//! 上传流程的小工具：本地文件身份、断点匹配、名字 / 大小校验与错误分类。

use std::collections::BTreeMap;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant, UNIX_EPOCH};

use chrono::Utc;

use crate::naming::MAX_NAME_BYTES;
use crate::upload_state::{
    UploadCredentials, UploadError, UploadPart, UploadResume, UPLOAD_RESUME_VERSION,
};
use crate::Error;

use super::{MAX_MULTIPART_PARTS, SMALL_FILE_LIMIT};

/// 上传前读到的本地文件身份（大小 + 修改时间）。
pub(super) struct LocalFileInfo {
    pub(super) size: u64,
    pub(super) modified_unix_secs: i64,
    pub(super) modified_nanos: u32,
}

pub(super) fn local_file_info(path: &Path) -> Result<LocalFileInfo, Error> {
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

pub(super) fn resume_matches(state: &UploadResume, local: &LocalFileInfo, parent_id: &str) -> bool {
    state.version == UPLOAD_RESUME_VERSION
        && state.parent_id == parent_id.trim()
        && state.size == local.size
        && state.local_modified_unix_secs == local.modified_unix_secs
        && state.local_modified_nanos == local.modified_nanos
        && (state.content_uploaded || !credentials_expired(&state.credentials))
}

pub(super) fn credentials_expired(credentials: &UploadCredentials) -> bool {
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

pub(super) fn part_length(part_number: u64, part_size: u64, size: u64) -> u64 {
    if part_number == 0 || part_size == 0 {
        return 0;
    }
    let offset = (part_number - 1).saturating_mul(part_size);
    size.saturating_sub(offset).min(part_size)
}

pub(super) fn completed_part_bytes(
    parts: &BTreeMap<u64, UploadPart>,
    part_size: u64,
    size: u64,
) -> u64 {
    parts
        .keys()
        .map(|number| part_length(*number, part_size, size))
        .sum()
}

pub(super) fn valid_completed_part(
    part: &UploadPart,
    part_count: u64,
    part_size: u64,
    size: u64,
) -> bool {
    part.number > 0
        && part.number <= part_count
        && !part.etag.trim().is_empty()
        && part.size == part_length(part.number, part_size, size)
}

pub(super) fn multipart_part_size(size: u64) -> u64 {
    let mut part = SMALL_FILE_LIMIT.max(1);
    while size.div_ceil(part) > MAX_MULTIPART_PARTS {
        part = part.saturating_mul(2);
    }
    part
}

pub(super) fn validate_name(name: &str) -> Result<(), Error> {
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

pub(super) fn ensure_file_size(path: &Path, expected: u64) -> Result<(), Error> {
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

pub(super) fn is_hard_auth_error(error: &Error) -> bool {
    matches!(
        error,
        Error::Auth { .. } | Error::Captcha { .. } | Error::RateLimited { .. }
    )
}

pub(super) fn is_retryable_api_error(error: &Error) -> bool {
    matches!(error, Error::Network(_))
        || matches!(
            error,
            Error::Api { code, .. } if matches!(*code, 408 | 425 | 500 | 502 | 503 | 504)
        )
}

pub(super) fn keep_resume_for_api_error(error: &Error) -> bool {
    matches!(
        error,
        Error::Network(_) | Error::RateLimited { .. } | Error::Auth { .. } | Error::Captcha { .. }
    ) || matches!(
        error,
        Error::Api { code, .. } if matches!(*code, 408 | 425 | 500 | 502 | 503 | 504)
    )
}

pub(super) fn preserve_after_error(error: Error) -> UploadError {
    if is_hard_auth_error(&error) {
        UploadError::hard(error)
    } else {
        UploadError::retryable(error)
    }
}

pub(super) fn ensure_not_cancelled<C>(cancelled: &C) -> Result<(), Error>
where
    C: Fn() -> bool,
{
    if cancelled() {
        Err(Error::Other("上传已取消".to_owned()))
    } else {
        Ok(())
    }
}

pub(super) fn sleep_cancellable<C>(duration: Duration, cancelled: &C) -> Result<(), Error>
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

pub(super) fn non_empty(value: String) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

pub(super) fn non_empty_name(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}
