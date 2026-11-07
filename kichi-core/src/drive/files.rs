//! 文件 / 最近修改 / 回收站列表，查询参数与条目 DTO。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer};

use crate::types::{EntryKind, FileEntry, TrashEntry};
use crate::Error;

use super::account::lenient_u64;
use super::media::{is_audio, is_image, is_video};
use super::{DriveClient, LIST_FILTERS, LIST_LIMIT, TRASH_FILTERS, TRASH_PARENT_ID};

impl DriveClient {
    /// 列出一个文件夹（`parent_id` 为 `None` 表示根目录）。
    ///
    /// 一次返回 `LIST_LIMIT` 条，文件和文件夹混在一起，用 [`EntryKind`] 区分；
    /// `next_page_token` 非空时表示还有下一页。
    pub fn list(
        &self,
        parent_id: Option<&str>,
        page_token: Option<&str>,
    ) -> Result<(Vec<FileEntry>, Option<String>), Error> {
        let query = list_query(parent_id, page_token);
        let response: FileListResponse = self.get_with_query(&self.config.files_url(), &query)?;
        if !response.kind.is_empty() && response.kind != "drive#fileList" {
            return Err(Error::ApiChanged(format!(
                "文件列表返回了意外的类型：{}",
                response.kind
            )));
        }
        let files = response
            .files
            .into_iter()
            .map(FileDto::into_entry)
            .collect();
        let next = (!response.next_page_token.is_empty()).then_some(response.next_page_token);
        Ok((files, next))
    }

    /// 全盘「最近修改」第一页（`parent_id=*`，canary 探针用）。
    ///
    /// 尝试带 `order=modified_time desc`，但**不能假设官方一定认这个参数**
    /// （未知查询参数历史上会被静默忽略）。调用方必须校验返回页确实按
    /// `modified_time` 倒序，校验不过就把探针当作不可用，不能据此重建索引。
    pub fn list_recent_page(
        &self,
        page_token: Option<&str>,
    ) -> Result<(Vec<FileEntry>, Option<String>), Error> {
        let response: FileListResponse =
            self.get_with_query(&self.config.files_url(), &recent_query(page_token))?;
        if !response.kind.is_empty() && response.kind != "drive#fileList" {
            return Err(Error::ApiChanged(format!(
                "最近修改列表返回了意外的类型：{}",
                response.kind
            )));
        }
        let files = response
            .files
            .into_iter()
            .map(FileDto::into_entry)
            .collect();
        let next = (!response.next_page_token.is_empty()).then_some(response.next_page_token);
        Ok((files, next))
    }

    /// 只读读取一个条目的完整列表形状。
    ///
    /// 播放历史通常随事件带 `reference_resource`；只有资源缺少名称或类型时，
    /// 播放历史页才用这个接口补一次。与播放用的 [`DriveClient::file_detail`]
    /// 不同，这里保留文件夹 / 文件、父目录和缩略图等表格字段。
    pub fn entry_detail(&self, id: &str) -> Result<FileEntry, Error> {
        let id = id.trim();
        if id.is_empty() {
            return Err(Error::InvalidInput("文件 id 为空".into()));
        }
        let dto: FileDto = self.get(&self.config().file_url(id))?;
        dto.into_complete_entry(id)
            .ok_or_else(|| Error::ApiChanged("文件详情缺少可用的 id、名称或类型".to_owned()))
    }

    /// 列出全账号回收站（自动包含各个子目录里的条目）。
    ///
    /// 必须同时使用 `parent_id=*` 和 `trashed=true`：只带后者时官方只返回
    /// 根目录下直接删除的条目。
    pub fn list_trash(
        &self,
        page_token: Option<&str>,
    ) -> Result<(Vec<TrashEntry>, Option<String>), Error> {
        let query = trash_query(page_token);
        let response: FileListResponse = self.get_with_query(&self.config.files_url(), &query)?;
        if !response.kind.is_empty() && response.kind != "drive#fileList" {
            return Err(Error::ApiChanged(format!(
                "回收站返回了意外的类型：{}",
                response.kind
            )));
        }
        let files = response
            .files
            .into_iter()
            .map(FileDto::into_trash_entry)
            .collect();
        let next = (!response.next_page_token.is_empty()).then_some(response.next_page_token);
        Ok((files, next))
    }
}

/// 文件列表的查询参数（单独成函数便于单测）。
pub(super) fn list_query(
    parent_id: Option<&str>,
    page_token: Option<&str>,
) -> Vec<(String, String)> {
    let mut query = vec![
        ("limit".to_owned(), LIST_LIMIT.to_string()),
        ("thumbnail_size".to_owned(), "SIZE_MEDIUM".to_owned()),
        ("with_audit".to_owned(), "true".to_owned()),
        ("filters".to_owned(), LIST_FILTERS.to_owned()),
    ];
    if let Some(parent) = parent_id.map(str::trim).filter(|id| !id.is_empty()) {
        query.push(("parent_id".to_owned(), parent.to_owned()));
    }
    if let Some(token) = page_token.map(str::trim).filter(|token| !token.is_empty()) {
        query.push(("page_token".to_owned(), token.to_owned()));
    }
    query
}

/// 「最近修改」查询参数：与全盘列举同一组过滤，额外尝试按修改时间倒序。
pub(super) fn recent_query(page_token: Option<&str>) -> Vec<(String, String)> {
    let mut query = vec![
        ("limit".to_owned(), LIST_LIMIT.to_string()),
        ("thumbnail_size".to_owned(), "SIZE_MEDIUM".to_owned()),
        ("with_audit".to_owned(), "true".to_owned()),
        ("filters".to_owned(), LIST_FILTERS.to_owned()),
        ("parent_id".to_owned(), "*".to_owned()),
        ("order".to_owned(), "modified_time desc".to_owned()),
    ];
    if let Some(token) = page_token.map(str::trim).filter(|token| !token.is_empty()) {
        query.push(("page_token".to_owned(), token.to_owned()));
    }
    query
}

/// 回收站查询参数。单独成函数，测试能直接锁住这两个容易漏的条件。
pub(super) fn trash_query(page_token: Option<&str>) -> Vec<(String, String)> {
    let mut query = vec![
        ("limit".to_owned(), LIST_LIMIT.to_string()),
        ("thumbnail_size".to_owned(), "SIZE_MEDIUM".to_owned()),
        ("with_audit".to_owned(), "true".to_owned()),
        ("filters".to_owned(), TRASH_FILTERS.to_owned()),
        ("parent_id".to_owned(), TRASH_PARENT_ID.to_owned()),
    ];
    if let Some(token) = page_token.map(str::trim).filter(|token| !token.is_empty()) {
        query.push(("page_token".to_owned(), token.to_owned()));
    }
    query
}

#[derive(Deserialize)]
pub(super) struct FileListResponse {
    #[serde(default)]
    pub(super) kind: String,
    #[serde(default)]
    pub(super) files: Vec<FileDto>,
    #[serde(default)]
    pub(super) next_page_token: String,
}

/// 官方文件条目（目录列表与全盘搜索共用）。
///
/// `pub(crate)`：`drive` / `search` / `tasks` 共用，不对外暴露 DTO。
#[derive(Deserialize)]
pub(crate) struct FileDto {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    kind: String,
    #[serde(default, deserialize_with = "lenient_u64")]
    size: u64,
    #[serde(default, deserialize_with = "lenient_time")]
    created_time: Option<DateTime<Utc>>,
    #[serde(default, deserialize_with = "lenient_time")]
    modified_time: Option<DateTime<Utc>>,
    #[serde(default, deserialize_with = "lenient_time")]
    delete_time: Option<DateTime<Utc>>,
    #[serde(default)]
    md5_checksum: String,
    #[serde(default)]
    audit: Option<AuditDto>,
    #[serde(default)]
    file_category: String,
    #[serde(default)]
    mime_type: String,
    #[serde(default)]
    file_extension: String,
    #[serde(default)]
    thumbnail_link: String,
    /// 所在文件夹 id：全盘列举时靠它还原真实路径；根目录为空串，未下发为 None。
    #[serde(default)]
    parent_id: Option<String>,
    /// 详情接口里不可靠；列表同时兼容显式布尔值和 `tags[].name == "STAR"`。
    #[serde(default)]
    starred: bool,
    #[serde(default, deserialize_with = "lenient_tags")]
    tags: Vec<FileTagDto>,
}

#[derive(Deserialize)]
struct FileTagDto {
    #[serde(default)]
    name: String,
}

/// 审核状态 DTO：列表与详情共用（`media.rs` 的详情也要读它）。
#[derive(Deserialize)]
pub(crate) struct AuditDto {
    #[serde(default)]
    pub(crate) status: String,
}

/// 审核状态是否异常：空状态按「未下发」处理，不算异常。
///
/// 列表的 `FileEntry::audit_flagged` 与详情的 `FileInfo::audit_flagged()`
/// 必须共用这一条，避免两处判定漂移。
pub(crate) fn audit_flagged(status: &str) -> bool {
    !status.is_empty() && !status.eq_ignore_ascii_case("STATUS_OK")
}

impl FileDto {
    /// 事件参考资源的 id；缺失时为空串。
    pub(crate) fn event_resource_id(&self) -> &str {
        self.id.trim()
    }

    /// 所在文件夹 id；缺失时为空串。
    pub(crate) fn parent_id(&self) -> Option<String> {
        self.parent_id.as_deref().map(str::trim).map(str::to_owned)
    }

    /// 转换成领域类型。
    pub(crate) fn into_entry(self) -> FileEntry {
        let is_folder = self.kind == "drive#folder";
        let starred = self.starred || self.tags.iter().any(|tag| tag.name == "STAR");
        let video = !is_folder
            && is_video(
                &self.name,
                &self.file_category,
                &self.mime_type,
                &self.file_extension,
            );
        let media = !is_folder
            && (video
                || is_audio(
                    &self.name,
                    &self.file_category,
                    &self.mime_type,
                    &self.file_extension,
                ));
        let has_visual_thumbnail = !is_folder
            && !self.thumbnail_link.trim().is_empty()
            && (video
                || is_image(
                    &self.name,
                    &self.file_category,
                    &self.mime_type,
                    &self.file_extension,
                ));
        FileEntry {
            id: self.id,
            name: self.name,
            kind: if is_folder {
                EntryKind::Folder
            } else {
                EntryKind::File
            },
            size: self.size,
            created_at: self.created_time,
            modified_at: self.modified_time,
            parent_id: self.parent_id.as_deref().unwrap_or("").trim().to_owned(),
            md5: (!self.md5_checksum.is_empty()).then_some(self.md5_checksum),
            audit_flagged: self.audit.is_some_and(|audit| audit_flagged(&audit.status)),
            starred,
            media,
            video,
            thumbnail_url: if has_visual_thumbnail {
                self.thumbnail_link.trim().to_owned()
            } else {
                String::new()
            },
        }
    }

    /// 事件里的 `reference_resource` 只有在带上名称和可识别类型时才足以直接
    /// 生成表格行；缺少这些字段时必须回退到只读详情请求，不能把未知类型猜成文件。
    ///
    /// `fallback_id` 来自事件自身的 `file_id`。资源偶尔不回传 id，这不影响
    /// 展示或打开，仍以事件的文件 id 为准；显式 id 与事件不一致时不能贴错文件，
    /// 必须返回 `None` 让调用方补详情。
    pub(crate) fn into_complete_entry(mut self, fallback_id: &str) -> Option<FileEntry> {
        let fallback_id = fallback_id.trim();
        let resource_id = self.id.trim();
        if fallback_id.is_empty()
            || (!resource_id.is_empty() && resource_id != fallback_id)
            || self.name.trim().is_empty()
            || EntryKind::from_api_kind(&self.kind).is_none()
        {
            return None;
        }
        self.id = fallback_id.to_owned();
        Some(self.into_entry())
    }

    /// 转换成回收站条目：保留原父目录与删除时间。
    pub(crate) fn into_trash_entry(self) -> TrashEntry {
        let parent_id = self.parent_id.as_deref().unwrap_or("").trim().to_owned();
        let deleted_at = self.delete_time;
        TrashEntry {
            entry: self.into_entry(),
            parent_id,
            deleted_at,
        }
    }

    /// 转换成搜索命中：条目 + 所在文件夹 id（全盘列举用）。
    pub(crate) fn into_hit(self) -> crate::search::FileHit {
        let parent_id = self.parent_id.as_deref().unwrap_or("").trim().to_owned();
        crate::search::FileHit {
            entry: self.into_entry(),
            parent_id,
        }
    }

    /// 单个条目自身：名字 + 父目录 id（拼「所在文件夹」路径用）。
    pub(crate) fn into_name_and_parent(self) -> (String, String) {
        (
            self.name,
            self.parent_id.as_deref().unwrap_or("").trim().to_owned(),
        )
    }
}

pub(crate) fn lenient_time<'de, D>(deserializer: D) -> Result<Option<DateTime<Utc>>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    let Some(text) = value else {
        return Ok(None);
    };
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    Ok(DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|time| time.with_timezone(&Utc)))
}

/// 官方标签通常是 `[{"name":"STAR"}]`；兼容 `null` 和纯字符串数组，
/// 未知形状直接忽略，不能因为标签字段让整个文件列表解析失败。
fn lenient_tags<'de, D>(deserializer: D) -> Result<Vec<FileTagDto>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    let Some(serde_json::Value::Array(values)) = value else {
        return Ok(Vec::new());
    };
    Ok(values
        .into_iter()
        .filter_map(|value| match value {
            serde_json::Value::Object(mut object) => object
                .remove("name")
                .and_then(|name| name.as_str().map(str::to_owned))
                .map(|name| FileTagDto { name }),
            serde_json::Value::String(name) => Some(FileTagDto { name }),
            _ => None,
        })
        .collect())
}
