//! 磁链资源列表与秒传。
//!
//! `POST /drive/v1/resource/list` 让服务端解析磁力链接并返回文件清单；
//! 带 GCID 的文件可以用 `POST /drive/v1/files` 直接秒传，不必先跑完整的
//! 云下载任务。端点与请求形状参照公开 Web 客户端的实际请求（只看行为，
//! 未复制代码）。
//!
//! 拆成两个职责：本文件是秒传入口与结果类型，[`magnet`] 只负责把
//! `resource/list` 响应整理成文件清单。

use serde_json::{json, Value};

use crate::http::Body;
use crate::naming::MAX_NAME_BYTES;
use crate::{Config, DriveClient, Error};

mod magnet;
#[cfg(test)]
mod tests;

use magnet::parse_magnet_listing;

/// 单个磁链最多接受的文件数；再多既不好选也不好逐一秒传。
pub const MAX_MAGNET_FILES: usize = 10_000;

/// 目录嵌套上限，防止异常响应把递归打穿。
const MAX_MAGNET_DEPTH: usize = 64;

/// 磁链里解析出的单个文件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MagnetFile {
    pub name: String,
    /// 相对保存容器根目录的文件夹链；空 = 直接放在目标目录。
    pub folders: Vec<String>,
    pub size: u64,
    /// 内容哈希；官方只在已缓存该内容时给出，缺失则无法秒传。
    pub gcid: Option<String>,
}

impl MagnetFile {
    pub fn available(&self) -> bool {
        self.gcid.is_some()
    }
}

/// 一次磁链解析的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MagnetListing {
    /// 单根目录时取目录名，否则取磁链 `dn=`，都没有时用「磁力任务」。
    pub root_name: String,
    pub files: Vec<MagnetFile>,
    pub total_size: u64,
}

impl MagnetListing {
    /// 已缓存内容、可以秒传的文件数。
    pub fn available_count(&self) -> usize {
        self.files.iter().filter(|file| file.available()).count()
    }
}

/// 秒传结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstantSave {
    /// 云端已有同样内容，条目已经直接建好。
    Saved,
    /// 内容不在云端缓存里，秒传不可用；只能改走普通云下载。
    Unavailable,
}

impl DriveClient {
    /// 解析磁力链接里的文件清单。
    ///
    /// `link` 必须是磁力链接或纯 BTIH；HTTP / ed2k 仍走普通离线任务，不在这里解析。
    pub fn list_magnet_resources(&self, link: &str) -> Result<MagnetListing, Error> {
        let link = crate::normalize_task_url(link);
        if link.is_empty() {
            return Err(Error::InvalidInput("磁力链接为空".into()));
        }
        if !crate::is_supported_task_url(&link)
            || !link.to_ascii_lowercase().starts_with("magnet:?")
        {
            return Err(Error::InvalidInput("不是可解析的磁力链接".into()));
        }
        let body = json!({ "page_size": 2000, "urls": link });
        let payload: Value = self.request(
            reqwest::Method::POST,
            &self.config().resource_list_url(),
            &[],
            Body::Json(&body),
        )?;
        parse_magnet_listing(&payload, &link)
    }

    /// 把一个已解析的文件秒传到 `parent_id` 下（空串 = 根目录）。
    ///
    /// 响应里若没有直接建好文件，官方可能已经插入了一个等待上传的空壳；
    /// 这里尽力把它移入回收站，避免文件列表里留下永远不动的条目。
    pub fn instant_save_file(
        &self,
        parent_id: &str,
        file: &MagnetFile,
    ) -> Result<InstantSave, Error> {
        let Some(gcid) = file
            .gcid
            .as_deref()
            .map(str::trim)
            .filter(|hash| !hash.is_empty())
        else {
            return Ok(InstantSave::Unavailable);
        };
        let name = sanitize_drive_name(&file.name, "未命名文件");
        let body = instant_save_body(parent_id, &name, file.size, gcid);
        let payload: Value = self.request(
            reqwest::Method::POST,
            &self.config().files_url(),
            &[],
            Body::Json(&body),
        )?;
        if instant_saved(&payload) {
            return Ok(InstantSave::Saved);
        }
        if let Some(id) = payload
            .get("file")
            .and_then(|file| file.get("id"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
        {
            // 清理失败不影响「不可秒传」这个结论，也不该盖住真正的失败原因。
            let trash = json!({ "ids": [id] });
            let _ = self.request_no_response_body(
                reqwest::Method::POST,
                &Config::file_action_url(self.config(), "batchTrash"),
                &[],
                &trash,
            );
        }
        Ok(InstantSave::Unavailable)
    }
}

/// 秒传请求体。hash 必须大写，size 是字符串。
fn instant_save_body(parent_id: &str, name: &str, size: u64, gcid: &str) -> Value {
    json!({
        "hash": gcid.trim().to_ascii_uppercase(),
        "name": name,
        "size": size.to_string(),
        "kind": "drive#file",
        "parent_id": parent_id.trim(),
        "upload_type": "UPLOAD_TYPE_RESUMABLE",
        "folder_type": "NORMAL",
        "objProvider": { "provider": "UPLOAD_TYPE_UNKNOWN" }
    })
}

/// 响应是否表示「内容已在云端，文件直接建好」。
///
/// 官方若给回 `resumable` 票据，说明它准备让你上传，内容其实没缓存；
/// 票据阶段是 PENDING / RUNNING 之类时同样不能当作成功。
fn instant_saved(payload: &Value) -> bool {
    let file = payload.get("file").filter(|value| value.is_object());
    let upload_type = payload
        .get("upload_type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_uppercase();
    let phase = file
        .and_then(|file| file.get("phase"))
        .or_else(|| payload.get("phase"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_uppercase();
    let has_resumable = payload
        .get("resumable")
        .is_some_and(|value| value.is_object());
    let pending = ["PENDING", "RUNNING", "PAUSED", "QUEUED", "PROCESSING"]
        .iter()
        .any(|word| phase.contains(word));
    let failed = ["FAILED", "ERROR", "CANCELLED", "CANCELED", "ABORTED"]
        .iter()
        .any(|word| phase.contains(word));
    let has_file_id = file
        .and_then(|file| file.get("id"))
        .and_then(Value::as_str)
        .is_some_and(|id| !id.trim().is_empty());

    !has_resumable
        && !pending
        && !failed
        && (upload_type == "UPLOAD_TYPE_UNKNOWN"
            || phase == "PHASE_TYPE_COMPLETE"
            || (file.is_some() && has_file_id))
}

pub fn sanitize_drive_name(raw: &str, fallback: &str) -> String {
    fn clean(value: &str) -> String {
        value
            .chars()
            .filter(|ch| {
                !ch.is_control()
                    && !matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
            })
            .collect::<String>()
            .trim()
            .trim_end_matches('.')
            .trim()
            .to_owned()
    }
    let mut name = clean(raw);
    if name.is_empty() {
        name = clean(fallback);
    }
    if name.is_empty() {
        name = "未命名".to_owned();
    }
    if name.len() > MAX_NAME_BYTES {
        let mut boundary = MAX_NAME_BYTES;
        while boundary > 0 && !name.is_char_boundary(boundary) {
            boundary -= 1;
        }
        name = name[..boundary].trim_end().to_owned();
        if name.is_empty() {
            name = "未命名".to_owned();
        }
    }
    name
}
