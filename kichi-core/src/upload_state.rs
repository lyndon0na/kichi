//! 可持久化的上传断点与上传错误。
//!
//! 这些类型只描述协议状态，不负责落盘；GUI 侧可以选择把 [`UploadResume`]
//! 放进 KWallet。临时 S3 凭据的 `Debug` 输出已脱敏，不能直接写日志。

use serde::{Deserialize, Serialize};

use crate::Error;

/// 断点格式版本；读取时保留未知字段，旧版本按默认值补齐。
pub const UPLOAD_RESUME_VERSION: u32 = 1;

/// 上传票据里的 S3 临时凭据。
///
/// 字段只在 `kichi-core` 内部和 serde 编解码时使用；GUI 只把它当作不透明
/// 的 [`UploadResume`] 一部分保存到 KWallet。
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct UploadCredentials {
    #[serde(default)]
    pub(crate) access_key_id: String,
    #[serde(default)]
    pub(crate) access_key_secret: String,
    #[serde(default)]
    pub(crate) security_token: String,
    #[serde(default)]
    pub(crate) bucket: String,
    #[serde(default)]
    pub(crate) key: String,
    #[serde(default)]
    pub(crate) endpoint: String,
    #[serde(default)]
    pub(crate) expiration: String,
}

impl std::fmt::Debug for UploadCredentials {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UploadCredentials")
            .field("bucket", &self.bucket)
            .field("key", &self.key)
            .field("endpoint", &self.endpoint)
            .field("expiration", &self.expiration)
            .field("access_key_id", &"<redacted>")
            .field("access_key_secret", &"<redacted>")
            .field("security_token", &"<redacted>")
            .finish()
    }
}

impl UploadCredentials {
    pub(crate) fn validate(&self) -> Result<(), Error> {
        let required = [
            ("access_key_id", self.access_key_id.trim()),
            ("access_key_secret", self.access_key_secret.trim()),
            ("bucket", self.bucket.trim()),
            ("key", self.key.trim()),
        ];
        if required.iter().any(|(_, value)| value.is_empty()) {
            return Err(Error::ApiChanged(
                "上传票据里的 S3 临时凭据不完整".to_owned(),
            ));
        }
        if !valid_bucket(self.bucket.trim()) {
            return Err(Error::ApiChanged("上传票据里的 bucket 无效".to_owned()));
        }
        Ok(())
    }
}

pub(crate) fn valid_bucket(bucket: &str) -> bool {
    !bucket.is_empty()
        && bucket.len() <= 63
        && bucket
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !bucket.starts_with('-')
        && !bucket.ends_with('-')
}

/// 已完成的一个 multipart 分片。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadPart {
    #[serde(default)]
    pub number: u64,
    #[serde(default)]
    pub etag: String,
    #[serde(default)]
    pub size: u64,
}

/// 一次可跨进程恢复的上传断点。
///
/// `content_uploaded=true` 表示 S3 内容已经提交完成，重启后只需继续轮询
/// 服务端任务；`upload_id` 非空时表示 multipart 已经创建，可按 `parts`
/// 继续补传。
#[derive(Clone, Serialize, Deserialize)]
pub struct UploadResume {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub parent_id: String,
    #[serde(default)]
    pub file_id: String,
    #[serde(default)]
    pub task_id: String,
    #[serde(default)]
    pub gcid: String,
    #[serde(default)]
    pub requested_name: String,
    #[serde(default)]
    pub final_name: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub local_modified_unix_secs: i64,
    #[serde(default)]
    pub local_modified_nanos: u32,
    #[serde(default)]
    pub credentials: UploadCredentials,
    #[serde(default)]
    pub upload_id: String,
    #[serde(default)]
    pub part_size: u64,
    /// 只在本进程内用于继续上传；跨进程恢复时由 S3 `ListParts` 重建。
    #[serde(skip, default)]
    pub parts: Vec<UploadPart>,
    #[serde(default)]
    pub bytes_sent: u64,
    #[serde(default)]
    pub content_uploaded: bool,
    /// 最后更新时间（Unix 秒），用于清理长期不用的断点。
    #[serde(default)]
    pub updated_unix: i64,
}

const fn default_version() -> u32 {
    UPLOAD_RESUME_VERSION
}

impl UploadResume {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        size: u64,
        local_modified_unix_secs: i64,
        local_modified_nanos: u32,
        parent_id: String,
        file_id: String,
        task_id: String,
        gcid: String,
        requested_name: String,
        final_name: String,
        credentials: UploadCredentials,
    ) -> Self {
        Self {
            version: UPLOAD_RESUME_VERSION,
            parent_id,
            file_id,
            task_id,
            gcid,
            requested_name,
            final_name,
            size,
            local_modified_unix_secs,
            local_modified_nanos,
            credentials,
            upload_id: String::new(),
            part_size: 0,
            parts: Vec::new(),
            bytes_sent: 0,
            content_uploaded: false,
            updated_unix: chrono::Utc::now().timestamp(),
        }
    }

    pub fn touch(&mut self) {
        self.updated_unix = chrono::Utc::now().timestamp();
    }
}

/// 上传失败时告诉调用方：这个错误是否值得保留断点、等用户手动重试。
#[derive(Debug)]
pub struct UploadError {
    pub error: Error,
    /// 保留 multipart / 任务断点，等用户手动重试。
    pub keep_resume: bool,
    /// 官方限制或鉴权失败：停止本轮上传队列，等用户处理后再手动开始。
    pub stop_queue: bool,
}

impl UploadError {
    pub(crate) fn fatal(error: Error) -> Self {
        let stop_queue = stops_upload_queue(&error);
        Self {
            error,
            keep_resume: false,
            stop_queue,
        }
    }

    pub(crate) fn retryable(error: Error) -> Self {
        Self {
            error,
            keep_resume: true,
            stop_queue: false,
        }
    }

    /// 不自动重试，但保留断点并停住整条上传队列（429 / 配额 / 鉴权）。
    pub(crate) fn hard(error: Error) -> Self {
        Self {
            error,
            keep_resume: true,
            stop_queue: true,
        }
    }
}

fn stops_upload_queue(error: &Error) -> bool {
    matches!(
        error,
        Error::Auth { .. } | Error::Captcha { .. } | Error::RateLimited { .. }
    )
}

impl From<Error> for UploadError {
    fn from(error: Error) -> Self {
        Self::fatal(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn credentials() -> UploadCredentials {
        UploadCredentials {
            access_key_id: "AKIA-SECRET-ID".into(),
            access_key_secret: "SECRET-VALUE".into(),
            security_token: "TOKEN-VALUE".into(),
            bucket: "bucket".into(),
            key: "key".into(),
            endpoint: "https://bucket.mypikpak.com".into(),
            expiration: "2099-01-01T00:00:00Z".into(),
        }
    }

    #[test]
    fn credentials_debug_never_prints_secrets() {
        let text = format!("{:?}", credentials());
        assert!(text.contains("bucket"));
        assert!(text.contains("<redacted>"));
        assert!(!text.contains("AKIA-SECRET-ID"));
        assert!(!text.contains("SECRET-VALUE"));
        assert!(!text.contains("TOKEN-VALUE"));
    }

    #[test]
    fn resume_roundtrip_keeps_compact_state_but_not_in_memory_parts() {
        let mut state = UploadResume::new(
            12,
            34,
            56,
            "parent".into(),
            "file".into(),
            "task".into(),
            "GCID".into(),
            "requested".into(),
            "final".into(),
            credentials(),
        );
        state.upload_id = "upload".into();
        state.part_size = 5;
        state.parts.push(UploadPart {
            number: 1,
            etag: "\"etag\"".into(),
            size: 5,
        });
        let json = serde_json::to_string(&state).expect("serialize");
        assert!(!json.contains("\"parts\""));
        let back: UploadResume = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.parent_id, "parent");
        assert_eq!(back.file_id, "file");
        assert_eq!(back.gcid, "GCID");
        assert!(back.parts.is_empty());
    }
}
