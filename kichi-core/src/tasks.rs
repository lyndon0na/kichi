//! 离线下载任务：创建 / 列表 / 删除。
//!
//! 端点行为已对照两个实现核对（均为可借用许可）：
//! - rclone `backend/pikpak`（MIT）实现了创建、单任务查询、删除，没有列表；
//! - pikpaktui `src/pikpak/offline.rs`（Apache-2.0）给出了列表的参数形态。
//! - pikpak-kotlin（MIT）记录了列表的 `with=reference_resource` 参数。
//!
//! 创建任务走的是 **文件**端点（`POST /drive/v1/files`），不是任务端点。

use serde::{Deserialize, Serialize};

use crate::drive::{is_media, is_video, DriveClient, FileDto};
use crate::http::Body;
use crate::types::{OfflineTask, TaskPhase};
use crate::Error;

mod status;
mod task_wait;

pub use status::TaskStatus;

/// 单页最多返回的任务数。
pub const TASK_LIST_LIMIT: u32 = 500;

/// 轮询时要关心的阶段（其余阶段（如已暂停/未知）不主动查询）。
pub const TASK_FILTER_PHASES: &str = "PHASE_TYPE_RUNNING,\
PHASE_TYPE_PENDING,\
PHASE_TYPE_COMPLETE,\
PHASE_TYPE_ERROR,\
PHASE_TYPE_PAUSED,\
PHASE_TYPE_UNKNOW";

/// 新建离线任务的请求体。
#[derive(Debug, Serialize)]
struct NewTaskRequest<'a> {
    kind: &'static str,
    upload_type: &'static str,
    url: UrlBody<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_id: Option<&'a str>,
    /// 只有未指定目录时才用 `DOWNLOAD` 指代官方默认下载目录。
    #[serde(skip_serializing_if = "Option::is_none")]
    folder_type: Option<&'static str>,
}

#[derive(Debug, Serialize)]
struct UrlBody<'a> {
    url: &'a str,
}

#[derive(Deserialize)]
struct NewTaskResponse {
    #[serde(default)]
    task: Option<TaskDto>,
}

#[derive(Deserialize)]
struct TaskListResponse {
    #[serde(default)]
    tasks: Vec<TaskDto>,
    #[serde(default)]
    next_page_token: String,
}

/// 官方任务对象（只取界面与轮询需要的字段）。
#[derive(Deserialize)]
struct TaskDto {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    phase: String,
    #[serde(default)]
    progress: i64,
    #[serde(default)]
    file_id: Option<String>,
    #[serde(default)]
    file_size: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    reference_resource: Option<FileDto>,
    #[serde(default, deserialize_with = "lenient_time")]
    created_time: Option<chrono::DateTime<chrono::Utc>>,
}

impl TaskDto {
    fn into_task(self) -> OfflineTask {
        let TaskDto {
            id,
            name,
            phase,
            progress,
            file_id,
            file_size,
            message,
            reference_resource,
            created_time,
        } = self;
        let output_parent_id = reference_resource.as_ref().and_then(FileDto::parent_id);
        let resource = reference_resource.map(FileDto::into_entry);
        let name = if name.trim().is_empty() {
            resource
                .as_ref()
                .map(|entry| entry.name.clone())
                .unwrap_or(name)
        } else {
            name
        };
        let progress = progress.clamp(0, 100) as u8;
        let parsed_size = file_size
            .as_deref()
            .and_then(|size| size.trim().parse::<u64>().ok())
            .unwrap_or(0);
        let file_size = if parsed_size > 0 {
            parsed_size
        } else {
            resource.as_ref().map(|entry| entry.size).unwrap_or(0)
        };
        let kind = resource.as_ref().map(|entry| entry.kind);
        let file_id = file_id.filter(|id| !id.is_empty()).or_else(|| {
            resource
                .as_ref()
                .map(|entry| entry.id.clone())
                .filter(|id| !id.is_empty())
        });
        let thumbnail_url = resource
            .as_ref()
            .map(|entry| entry.thumbnail_url.clone())
            .unwrap_or_default();
        let video = resource
            .as_ref()
            .map(|entry| entry.video)
            .unwrap_or_else(|| is_video(&name, "", "", ""));
        let media = resource
            .as_ref()
            .map(|entry| entry.media)
            .unwrap_or_else(|| is_media(&name, "", "", ""));
        OfflineTask {
            id,
            name,
            phase: phase_from(&phase),
            progress,
            message: message.filter(|msg| !msg.is_empty()),
            created_at: created_time,
            file_id,
            output_parent_id,
            file_size,
            kind,
            thumbnail_url,
            media,
            video,
        }
    }
}

/// 官方 `PHASE_TYPE_*` → 领域枚举；未识别的取值归 `Unknown`。
fn phase_from(raw: &str) -> TaskPhase {
    match raw {
        "PHASE_TYPE_PENDING" => TaskPhase::Pending,
        "PHASE_TYPE_RUNNING" => TaskPhase::Running,
        "PHASE_TYPE_PAUSED" => TaskPhase::Paused,
        "PHASE_TYPE_COMPLETE" => TaskPhase::Completed,
        "PHASE_TYPE_ERROR" => TaskPhase::Error,
        _ => TaskPhase::Unknown,
    }
}

/// RFC3339 时间；官方可能给 `null`、空串或无法解析的值，一律按“没有时间”处理。
fn lenient_time<'de, D>(deserializer: D) -> Result<Option<chrono::DateTime<chrono::Utc>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    let Some(text) = value else {
        return Ok(None);
    };
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    Ok(chrono::DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|time| time.with_timezone(&chrono::Utc)))
}

/// `None` 使用官方默认下载目录；`Some("")` 表示根目录；`Some(id)` 表示指定目录。
fn new_task_request<'a>(url: &'a str, parent_id: Option<&'a str>) -> NewTaskRequest<'a> {
    match parent_id.map(str::trim) {
        None => NewTaskRequest {
            kind: "drive#file",
            upload_type: "UPLOAD_TYPE_URL",
            url: UrlBody { url },
            parent_id: None,
            folder_type: Some("DOWNLOAD"),
        },
        Some("") => NewTaskRequest {
            kind: "drive#file",
            upload_type: "UPLOAD_TYPE_URL",
            url: UrlBody { url },
            parent_id: None,
            folder_type: None,
        },
        Some(parent_id) => NewTaskRequest {
            kind: "drive#file",
            upload_type: "UPLOAD_TYPE_URL",
            url: UrlBody { url },
            parent_id: Some(parent_id),
            folder_type: None,
        },
    }
}

impl DriveClient {
    /// 新建离线下载任务（磁力 / ed2k / 普通 URL 均可）。
    ///
    /// `parent_id = None` 使用官方默认下载目录；`Some("")` 表示根目录。
    pub fn create_offline_task(
        &self,
        url: &str,
        parent_id: Option<&str>,
    ) -> Result<OfflineTask, Error> {
        let url = url.trim();
        if url.is_empty() {
            return Err(Error::InvalidInput("下载链接为空".into()));
        }
        let body = new_task_request(url, parent_id);
        let payload = serde_json::to_value(&body)
            .map_err(|e| Error::ApiChanged(format!("任务请求体无法序列化：{e}")))?;
        let response: NewTaskResponse = self.request(
            reqwest::Method::POST,
            &self.config().files_url(),
            &[],
            Body::Json(&payload),
        )?;
        response
            .task
            .map(TaskDto::into_task)
            .ok_or_else(|| Error::ApiChanged("新建任务响应里没有 task".into()))
    }

    /// 读取一页离线任务（`page_token` 为 `None` 表示第一页）。
    pub fn list_offline_tasks(
        &self,
        page_token: Option<&str>,
    ) -> Result<(Vec<OfflineTask>, Option<String>), Error> {
        let query = task_list_query(page_token);
        let response: TaskListResponse = self.get_with_query(&self.config().tasks_url(), &query)?;
        let tasks = response.tasks.into_iter().map(TaskDto::into_task).collect();
        let next = (!response.next_page_token.is_empty()).then_some(response.next_page_token);
        Ok((tasks, next))
    }

    /// 删除任务记录。`delete_files` 为 `false` 时只删任务，云端产物保留。
    pub fn delete_offline_task(&self, id: &str, delete_files: bool) -> Result<(), Error> {
        let id = id.trim();
        if id.is_empty() {
            return Err(Error::InvalidInput("任务 id 为空".into()));
        }
        let query = vec![
            ("delete_files".to_owned(), delete_files.to_string()),
            ("task_ids".to_owned(), id.to_owned()),
        ];
        self.request_no_response(reqwest::Method::DELETE, &self.config().tasks_url(), &query)
    }
}

/// 任务列表的查询参数（单独成函数便于单测）。
fn task_list_query(page_token: Option<&str>) -> Vec<(String, String)> {
    let filters = format!(r#"{{"phase":{{"in":"{TASK_FILTER_PHASES}"}}}}"#);
    let mut query = vec![
        ("type".to_owned(), "offline".to_owned()),
        ("thumbnail_size".to_owned(), "SIZE_SMALL".to_owned()),
        ("limit".to_owned(), TASK_LIST_LIMIT.to_string()),
        ("filters".to_owned(), filters),
        ("with".to_owned(), "reference_resource".to_owned()),
    ];
    if let Some(token) = page_token.map(str::trim).filter(|token| !token.is_empty()) {
        query.push(("page_token".to_owned(), token.to_owned()));
    }
    query
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::EntryKind;

    #[test]
    fn phase_mapping_covers_documented_and_unknown_values() {
        assert_eq!(phase_from("PHASE_TYPE_PENDING"), TaskPhase::Pending);
        assert_eq!(phase_from("PHASE_TYPE_RUNNING"), TaskPhase::Running);
        assert_eq!(phase_from("PHASE_TYPE_PAUSED"), TaskPhase::Paused);
        assert_eq!(phase_from("PHASE_TYPE_COMPLETE"), TaskPhase::Completed);
        assert_eq!(phase_from("PHASE_TYPE_ERROR"), TaskPhase::Error);
        assert_eq!(phase_from("PHASE_TYPE_UNKNOW"), TaskPhase::Unknown);
        assert_eq!(phase_from(""), TaskPhase::Unknown);
    }

    #[test]
    fn list_query_matches_pikpaktui_parameters() {
        let query = task_list_query(None);
        let get = |key: &str| {
            query
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(get("type"), Some("offline"));
        assert_eq!(get("thumbnail_size"), Some("SIZE_SMALL"));
        assert_eq!(get("limit"), Some("500"));
        assert_eq!(get("with"), Some("reference_resource"));
        assert!(get("page_token").is_none());
        let filters = get("filters").unwrap_or_default();
        assert!(filters.starts_with(r#"{"phase":{"in":"#));
        assert!(filters.contains("PHASE_TYPE_RUNNING"));
        assert!(filters.contains("PHASE_TYPE_COMPLETE"));

        let paged = task_list_query(Some("tok"));
        assert!(paged.contains(&("page_token".to_owned(), "tok".to_owned())));
    }

    #[test]
    fn task_parses_string_size_and_optional_fields() {
        let json = r#"{
            "tasks": [
                {"id":"T1","name":"电影","phase":"PHASE_TYPE_RUNNING","progress":42,
                 "file_size":"123456","file_id":"F1","message":"downloading",
                 "reference_resource":{"id":"D1","name":"电影","kind":"drive#folder",
                 "parent_id":"folder-9"},
                 "created_time":"2026-10-20T12:34:56.789+08:00"},
                {"id":"T2","name":"done.zip","phase":"PHASE_TYPE_COMPLETE",
                 "progress":100,"file_size":null,"message":"",
                 "reference_resource":{"id":"F2","name":"done.zip","kind":"drive#file",
                 "size":"42","file_category":"VIDEO","mime_type":"video/mp4",
                 "thumbnail_link":"https://thumb.invalid/done.jpg"}},
                {"id":"T3","name":"unknown","phase":"PHASE_TYPE_COMPLETE",
                 "progress":100,"file_size":"0"},
                {"id":"T4","name":"track.flac","phase":"PHASE_TYPE_COMPLETE",
                 "progress":100,"file_id":"F4","file_size":"7"}
            ],
            "next_page_token": "tok"
        }"#;
        let parsed: TaskListResponse = serde_json::from_str(json).expect("任务列表应可解析");
        assert_eq!(parsed.next_page_token, "tok");
        let tasks: Vec<OfflineTask> = parsed.tasks.into_iter().map(TaskDto::into_task).collect();

        assert_eq!(tasks[0].phase, TaskPhase::Running);
        assert_eq!(tasks[0].progress, 42);
        assert_eq!(tasks[0].file_size, 123_456, "字符串 file_size 应可解析");
        assert_eq!(tasks[0].file_id.as_deref(), Some("F1"));
        assert_eq!(tasks[0].output_parent_id.as_deref(), Some("folder-9"));
        assert_eq!(tasks[0].message.as_deref(), Some("downloading"));
        assert_eq!(tasks[0].kind, Some(EntryKind::Folder));
        assert!(!tasks[0].media);
        assert!(!tasks[0].video);
        assert_eq!(
            tasks[0].created_at.map(|t| t.timestamp()),
            Some(1_792_470_896)
        );

        assert_eq!(tasks[1].phase, TaskPhase::Completed);
        assert_eq!(
            tasks[1].output_parent_id, None,
            "资源缺少 parent_id 时不能冒充根目录"
        );
        assert_eq!(tasks[1].file_size, 42, "顶层无大小时回退产物大小");
        assert_eq!(tasks[1].message, None, "空说明应为 None");
        assert_eq!(tasks[1].created_at, None);
        assert_eq!(tasks[1].kind, Some(EntryKind::File));
        assert_eq!(
            tasks[1].file_id.as_deref(),
            Some("F2"),
            "顶层缺 file_id 时回退 reference_resource.id"
        );
        assert_eq!(tasks[1].thumbnail_url, "https://thumb.invalid/done.jpg");
        assert!(tasks[1].media);
        assert!(tasks[1].video);
        assert_eq!(tasks[2].kind, None, "缺 reference_resource 时应兜底");
        assert!(!tasks[2].media);
        assert!(!tasks[2].video);
        assert!(tasks[3].media, "资源缺失时仍按产物后缀识别音频");
        assert!(!tasks[3].video);
    }

    #[test]
    fn new_task_request_uses_default_download_directory() {
        let body = new_task_request("magnet:?xt=urn:btih:abc", None);
        let value = serde_json::to_value(&body).expect("请求体应可序列化");
        assert_eq!(value["kind"], "drive#file");
        assert_eq!(value["upload_type"], "UPLOAD_TYPE_URL");
        assert_eq!(value["url"]["url"], "magnet:?xt=urn:btih:abc");
        assert_eq!(value["folder_type"], "DOWNLOAD");
        assert!(value.get("parent_id").is_none(), "默认目录不应带 parent_id");
    }

    #[test]
    fn new_task_request_targets_explicit_folder() {
        let body = new_task_request("https://example.com/file.zip", Some(" folder-1 "));
        let value = serde_json::to_value(&body).expect("请求体应可序列化");
        assert_eq!(value["parent_id"], "folder-1");
        assert!(
            value.get("folder_type").is_none(),
            "指定目录时必须省略 folder_type"
        );
    }

    #[test]
    fn new_task_request_targets_root_without_download_folder_type() {
        let body = new_task_request("https://example.com/file.zip", Some(""));
        let value = serde_json::to_value(&body).expect("请求体应可序列化");
        assert!(value.get("parent_id").is_none(), "根目录不传 parent_id");
        assert!(
            value.get("folder_type").is_none(),
            "根目录不能退回官方默认下载目录"
        );
    }

    #[test]
    fn empty_url_is_rejected_before_network() {
        // 构造一个不做 I/O 的驱动：只验证参数校验分支
        let config = crate::Config::default();
        let client = DriveClient::new(&config, "0123456789abcdef0123456789abcdef", "", "")
            .expect("客户端应可构造");
        assert!(matches!(
            client.create_offline_task("   ", None),
            Err(Error::InvalidInput(_))
        ));
        assert!(matches!(
            client.delete_offline_task("", false),
            Err(Error::InvalidInput(_))
        ));
    }
}
