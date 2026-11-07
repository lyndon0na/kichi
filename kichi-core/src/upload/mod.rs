//! 上传：CID / GCID、上传票据、S3 SigV4、单段 / 分片上传与服务端任务确认。
//!
//! 上传票据里的临时 access key、secret 和 security token 只活在本模块调用栈内，
//! 不进入日志、错误文案或任何持久化结构。
//!
//! 按职责拆分（守行数软上限）：入口与断点身份在 [`client`]，GCID 与传输编排在
//! [`transfer`]，服务端任务在 [`task`]，S3 在 [`s3`] / [`s3_request`] /
//! [`s3_error`] / [`s3_xml`]，哈希在 [`cid`]，读取器与编码在 [`io`]，
//! 其余小工具在 [`util`]。

mod cid;
mod client;
mod io;
mod s3;
mod s3_error;
mod s3_request;
mod s3_xml;
mod task;
#[cfg(test)]
mod tests;
mod transfer;
mod types;
mod util;

use std::time::Duration;

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
const S3_REQUEST_TIMEOUT: Duration = Duration::from_secs(300);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);
/// 同一个 multipart 文件最多同时发送几个分片。
///
/// 分片从本地文件流式读取，不再整块常驻内存；跨文件并发仍由 GUI 调度器
/// 控制，所有连接共享同一个上传限速桶。
const MAX_PARALLEL_PARTS: usize = 4;

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
    /// 上传阶段的实际进度：已完成分片 + 当前在途分片已读出的字节。
    pub bytes: u64,
    /// 已确认完成、可续传的字节；`None` = 当前阶段不改变记录里的断点进度。
    pub completed_bytes: Option<u64>,
    pub total: u64,
}

/// 上传成功后的服务端信息。文件名可能被官方自动追加编号。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadResult {
    pub file_id: String,
    pub name: String,
    pub bytes_sent: u64,
}
