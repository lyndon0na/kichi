//! 上传票据、服务端任务与清理用的内部 DTO。

use serde::Deserialize;

use crate::upload_state::UploadCredentials;

#[derive(Debug, Deserialize, Default)]
pub(super) struct UploadTicket {
    #[serde(default)]
    pub(super) file: TicketFile,
    #[serde(default)]
    pub(super) resumable: TicketResumable,
    #[serde(default)]
    pub(super) task: Option<TaskDto>,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct TicketFile {
    #[serde(default)]
    pub(super) id: String,
    #[serde(default)]
    pub(super) name: String,
    #[serde(default)]
    pub(super) phase: String,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct TicketResumable {
    #[serde(default)]
    pub(super) params: Option<UploadCredentials>,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct TaskDto {
    #[serde(default)]
    pub(super) id: String,
    #[serde(default)]
    pub(super) phase: String,
    #[serde(default)]
    pub(super) message: String,
    #[serde(default)]
    pub(super) file_id: String,
    #[serde(default)]
    pub(super) file_name: String,
}

#[derive(Debug, Default, Clone)]
pub(super) struct UploadCleanup {
    pub(super) file_id: String,
    pub(super) task_id: String,
    pub(super) upload_id: String,
    pub(super) credentials: Option<UploadCredentials>,
    pub(super) content_uploaded: bool,
}
