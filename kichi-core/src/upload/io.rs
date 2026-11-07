//! 上传请求体读取器与路径 / 摘要编码。

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Arc;

use crate::upload_rate::UploadRateLimiter;
use crate::Error;

pub(super) struct ByteSliceReader {
    bytes: Arc<Vec<u8>>,
    offset: usize,
}

impl ByteSliceReader {
    pub(super) fn new(bytes: Arc<Vec<u8>>) -> Self {
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

/// 只读取文件中一个固定区间；每次上传尝试都重新打开并 seek，重试不会复用
/// 已经被 reqwest 消费过的读者。
pub(super) struct FileRangeReader {
    file: File,
    remaining: u64,
}

impl FileRangeReader {
    pub(super) fn open(path: &Path, offset: u64, length: u64) -> Result<Self, Error> {
        let mut file =
            File::open(path).map_err(|err| Error::Other(format!("无法打开本地文件：{err}")))?;
        file.seek(SeekFrom::Start(offset))
            .map_err(|err| Error::Other(format!("无法定位本地文件：{err}")))?;
        Ok(Self {
            file,
            remaining: length,
        })
    }
}

impl Read for FileRangeReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.remaining == 0 || buffer.is_empty() {
            return Ok(0);
        }
        let limit =
            usize::try_from(self.remaining.min(buffer.len() as u64)).unwrap_or(buffer.len());
        let read = self.file.read(&mut buffer[..limit])?;
        self.remaining = self.remaining.saturating_sub(read as u64);
        Ok(read)
    }
}

/// 上传请求体读取器：S3 还在读取本地数据时也能看到取消信号。
///
/// `reqwest::blocking::Body::sized` 要求读取器 `'static`，所以取消判据本身
/// 必须是可克隆、线程安全且拥有所有权的闭包。
pub(super) struct CancelReader<R, C> {
    inner: R,
    cancelled: C,
}

impl<R, C> CancelReader<R, C>
where
    C: Fn() -> bool,
{
    pub(super) fn new(inner: R, cancelled: C) -> Self {
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
pub(super) struct RateLimitedReader<R, C> {
    inner: R,
    limiter: Option<Arc<UploadRateLimiter>>,
    cancelled: C,
}

impl<R, C> RateLimitedReader<R, C>
where
    C: Fn() -> bool,
{
    pub(super) fn new(inner: R, limiter: Option<Arc<UploadRateLimiter>>, cancelled: C) -> Self {
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

pub(super) fn encode_path(key: &str) -> String {
    let encoded = percent_encode_keep_slash(key.as_bytes());
    if encoded.starts_with('/') {
        encoded
    } else {
        format!("/{encoded}")
    }
}

pub(super) fn percent_encode_keep_slash(bytes: &[u8]) -> String {
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

pub(super) fn percent_encode(bytes: &[u8]) -> String {
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

pub(super) fn is_unreserved(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~')
}

pub(super) fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

pub(super) fn hex_upper(bytes: &[u8]) -> String {
    hex_lower(bytes).to_ascii_uppercase()
}
