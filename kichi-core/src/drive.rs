//! 网盘侧接口：容量 / 会员 / 当前用户 / 文件列表。
//!
//! 每个请求都带 `x-captcha-token`；官方以 `captcha_invalid` 拒绝时，
//! 作废并重取一次再试（rclone 同款兜底）。

use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer};

use crate::captcha::CaptchaSource;
use crate::http::{Body, HttpClient};
use crate::types::{EntryKind, FileEntry, Quota};
use crate::{Config, Error};

/// 单页最多返回的条目数（rclone 的 `ListLimit`）。
const LIST_LIMIT: u32 = 500;
/// 文件列表只取已完成、未进回收站的条目。
const LIST_FILTERS: &str = r#"{"phase":{"eq":"PHASE_TYPE_COMPLETE"},"trashed":{"eq":false}}"#;

/// 已登录的网盘客户端（内部令牌由调用方的 `Session` 提供）。
pub struct DriveClient {
    config: Config,
    http: HttpClient,
    captcha: CaptchaSource,
    access_token: Arc<str>,
    user_id: String,
}

/// 当前用户摘要。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct UserInfo {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub email: String,
}

#[derive(Deserialize)]
struct AboutResponse {
    #[serde(default)]
    quota: Option<AboutQuota>,
}

#[derive(Deserialize)]
struct AboutQuota {
    #[serde(default, deserialize_with = "lenient_u64")]
    usage: u64,
    #[serde(default, deserialize_with = "lenient_u64")]
    limit: u64,
    #[serde(default)]
    is_unlimited: bool,
}

/// 官方把大数字编码为字符串（`"123"`），也兼容直接给数字。
fn lenient_u64<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error as _;
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Number(n) => Ok(n.as_u64().unwrap_or(0)),
        serde_json::Value::String(s) => s.parse::<u64>().map_err(D::Error::custom),
        _ => Ok(0),
    }
}

impl DriveClient {
    fn with_http(config: Config, http: HttpClient, access_token: Arc<str>, user_id: &str) -> Self {
        let captcha = CaptchaSource::new(config.clone(), http.clone());
        Self {
            config,
            http,
            captcha,
            access_token,
            user_id: user_id.to_owned(),
        }
    }

    /// 已知令牌构造客户端（不做任何 I/O；登录/刷新由 `auth` 完成）。
    pub fn new(
        config: &Config,
        device_id: &str,
        access_token: &str,
        user_id: &str,
    ) -> Result<Self, Error> {
        let http = HttpClient::new(config.clone(), device_id)?;
        Ok(Self::with_http(
            config.clone(),
            http,
            Arc::from(access_token),
            user_id,
        ))
    }

    /// 带鉴权与 captcha 的 GET 请求（captcha 被拒时自动重试一次）。
    pub(crate) fn get<R: serde::de::DeserializeOwned>(&self, url: &str) -> Result<R, Error> {
        self.get_with_query(url, &[])
    }

    /// 同 [`DriveClient::get`]，额外附带查询参数。
    pub(crate) fn get_with_query<R: serde::de::DeserializeOwned>(
        &self,
        url: &str,
        query: &[(String, String)],
    ) -> Result<R, Error> {
        self.request(reqwest::Method::GET, url, query, Body::Empty)
    }

    /// 统一请求入口：取 captcha 令牌发一次，被拒时作废并重取后重试一次。
    pub(crate) fn request<R: serde::de::DeserializeOwned>(
        &self,
        method: reqwest::Method,
        url: &str,
        query: &[(String, String)],
        body: Body<'_>,
    ) -> Result<R, Error> {
        let action = action_for(method.as_str(), url);
        let token = self.captcha.acquire(&action, None, Some(&self.user_id))?;
        let access = Some(self.access_token.as_ref());
        let first = self.http.call_with_query::<R>(
            method.clone(),
            url,
            query,
            body.reborrow(),
            access,
            Some(token.token.as_str()).filter(|t| !t.is_empty()),
        );
        match first {
            Err(err) if err.is_captcha() => {
                self.captcha.invalidate();
                let retry = self.captcha.acquire(&action, None, Some(&self.user_id))?;
                self.http.call_with_query(
                    method,
                    url,
                    query,
                    body,
                    access,
                    Some(retry.token.as_str()).filter(|t| !t.is_empty()),
                )
            }
            other => other,
        }
    }

    /// 同 [`DriveClient::request`]，但响应体可能为空（DELETE 之类）。
    pub(crate) fn request_no_response(
        &self,
        method: reqwest::Method,
        url: &str,
        query: &[(String, String)],
    ) -> Result<(), Error> {
        let action = action_for(method.as_str(), url);
        let token = self.captcha.acquire(&action, None, Some(&self.user_id))?;
        let access = Some(self.access_token.as_ref());
        let first = self.http.call_no_response(
            method.clone(),
            url,
            query,
            access,
            Some(token.token.as_str()).filter(|t| !t.is_empty()),
        );
        match first {
            Err(err) if err.is_captcha() => {
                self.captcha.invalidate();
                let retry = self.captcha.acquire(&action, None, Some(&self.user_id))?;
                self.http.call_no_response(
                    method,
                    url,
                    query,
                    access,
                    Some(retry.token.as_str()).filter(|t| !t.is_empty()),
                )
            }
            other => other,
        }
    }

    /// 运行时配置（`tasks` 模块据此拼任务端点）。
    pub(crate) fn config(&self) -> &Config {
        &self.config
    }

    /// 容量与用量。
    pub fn about(&self) -> Result<Quota, Error> {
        let response: AboutResponse = self.get(&self.config.about_url())?;
        let quota = response
            .quota
            .ok_or_else(|| Error::ApiChanged("about 响应缺少 quota".into()))?;
        // 会员状态只在成功时覆盖，失败按非会员处理（不让容量查询一起失败）
        let vip = self.is_vip().unwrap_or(false);
        Ok(Quota {
            usage: quota.usage,
            limit: (!quota.is_unlimited && quota.limit > 0).then_some(quota.limit),
            vip,
        })
    }

    /// 会员状态（失败不致命，`about` 里按 false 处理）。
    pub fn is_vip(&self) -> Result<bool, Error> {
        let vip: VipResponse = self.get(&self.config.vip_url())?;
        Ok(vip.data.is_some_and(|data| {
            data.status.eq_ignore_ascii_case("ok") && !data.vip_type.eq_ignore_ascii_case("novip")
        }))
    }

    /// 当前用户（name / email 用来在界面上显示身份）。
    pub fn me(&self) -> Result<UserInfo, Error> {
        self.get(&self.config.user_me_url())
    }

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
}

/// 文件列表的查询参数（单独成函数便于单测）。
fn list_query(parent_id: Option<&str>, page_token: Option<&str>) -> Vec<(String, String)> {
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

#[derive(Deserialize)]
struct FileListResponse {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    files: Vec<FileDto>,
    #[serde(default)]
    next_page_token: String,
}

#[derive(Deserialize)]
struct FileDto {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    kind: String,
    #[serde(default, deserialize_with = "lenient_u64")]
    size: u64,
    #[serde(default, deserialize_with = "lenient_time")]
    modified_time: Option<DateTime<Utc>>,
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
}

#[derive(Deserialize)]
struct AuditDto {
    #[serde(default)]
    status: String,
}

impl FileDto {
    fn into_entry(self) -> FileEntry {
        let is_folder = self.kind == "drive#folder";
        let media = !is_folder
            && is_media(
                &self.name,
                &self.file_category,
                &self.mime_type,
                &self.file_extension,
            );
        FileEntry {
            id: self.id,
            name: self.name,
            kind: if is_folder {
                EntryKind::Folder
            } else {
                EntryKind::File
            },
            size: self.size,
            modified_at: self.modified_time,
            md5: (!self.md5_checksum.is_empty()).then_some(self.md5_checksum),
            audit_flagged: self.audit.is_some_and(|audit| {
                !audit.status.is_empty() && !audit.status.eq_ignore_ascii_case("STATUS_OK")
            }),
            media,
        }
    }
}

/// 视频/音频扩展名表（判断播放入口用）。
const MEDIA_EXTENSIONS: &[&str] = &[
    "mp4", "mkv", "avi", "mov", "webm", "ts", "m2ts", "m4v", "flv", "wmv", "rmvb", "rm", "mpg",
    "mpeg", "3gp", "ogv", "mp3", "flac", "wav", "m4a", "ogg", "opus", "aac", "ape", "wma",
];

/// 是否按媒体处理（决定是否显示播放入口）。
///
/// **实测（2026-10-26）**：文件列表接口在不带 `kind` 过滤时，
/// `file_category` / `mime_type` / `file_extension` 全是空串（带 `kind`
/// 过滤才下发），因此不能只信官方分类，必须按扩展名兜底，否则视频文件
/// 在界面上没有播放按钮。官方分类可用时优先采信。
pub(crate) fn is_media(name: &str, category: &str, mime: &str, extension: &str) -> bool {
    let heard = |text: &str| matches!(text.trim().to_ascii_uppercase().as_str(), "VIDEO" | "AUDIO");
    if heard(category) {
        return true;
    }
    if mime.trim().to_ascii_lowercase().starts_with("video/")
        || mime.trim().to_ascii_lowercase().starts_with("audio/")
    {
        return true;
    }
    let from_extension = |text: &str| {
        let ext = text.trim().trim_start_matches('.').to_ascii_lowercase();
        MEDIA_EXTENSIONS.contains(&ext.as_str())
    };
    if !extension.is_empty() && from_extension(extension) {
        return true;
    }
    // 扩展名带点或列表里两者都缺时，从文件名末段取
    name.rsplit_once('.')
        .is_some_and(|(_, ext)| from_extension(ext))
}

/// RFC3339 时间；官方可能给 `null`、空串或无法解析的值，一律按“没有时间”处理。
fn lenient_time<'de, D>(deserializer: D) -> Result<Option<DateTime<Utc>>, D::Error>
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

#[derive(Deserialize)]
struct VipResponse {
    #[serde(default)]
    data: Option<VipData>,
}

#[derive(Deserialize)]
struct VipData {
    #[serde(default)]
    status: String,
    #[serde(default, rename = "type")]
    vip_type: String,
}

/// 把 URL 还原成 rclone 风格的 action：`GET:/drive/v1/about`。
fn action_for(method: &str, url: &str) -> String {
    let path = url
        .split("://")
        .nth(1)
        .and_then(|rest| rest.find('/').map(|idx| &rest[idx..]))
        .unwrap_or(url);
    let path = path.split('?').next().unwrap_or(path);
    format!("{method}:{path}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_uses_path_only() {
        assert_eq!(
            action_for("GET", "https://api-drive.mypikpak.com/drive/v1/about"),
            "GET:/drive/v1/about"
        );
        assert_eq!(
            action_for("GET", "https://user.mypikpak.com/v1/user/me?x=1"),
            "GET:/v1/user/me"
        );
        assert_eq!(action_for("GET", "/drive/v1/about"), "GET:/drive/v1/about");
    }

    #[test]
    fn media_flag_survives_empty_official_category() {
        // 实测：不带 kind 过滤时官方分类字段为空，靠扩展名兜底
        assert!(is_media("Screencast_20261023.webm", "", "", ""));
        assert!(is_media("预告片.MP4", "", "", ""));
        assert!(is_media("song.flac", "", "", ""));
        assert!(is_media(".hidden.txt.mp3", "", "", ""));
        // 官方给了分类时优先采信（即使扩展名不认识）
        assert!(is_media("noext", "VIDEO", "", ""));
        assert!(is_media("file.bin", "", "video/mp4", ""));
        // 非媒体
        assert!(!is_media("settings.json", "", "", ""));
        assert!(!is_media("archive.zip", "", "", ""));
        assert!(!is_media("no-extension", "", "", ""));
        assert!(!is_media("fake.mp4.txt", "", "", ""));
    }

    #[test]
    fn quota_numbers_may_be_strings() {
        let json = r#"{"quota":{"limit":"10995116277760","usage":"123","is_unlimited":false}}"#;
        let Ok(parsed) = serde_json::from_str::<AboutResponse>(json) else {
            panic!("quota 字符串形式应可解析");
        };
        let Some(quota) = parsed.quota else {
            panic!("缺少 quota");
        };
        assert_eq!(quota.usage, 123);
        assert_eq!(quota.limit, 10_995_116_277_760);
    }

    #[test]
    fn list_query_matches_rclone_parameters() {
        let root = list_query(None, None);
        let get = |key: &str| root.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str());
        assert_eq!(get("limit"), Some("500"));
        assert_eq!(get("thumbnail_size"), Some("SIZE_MEDIUM"));
        assert_eq!(get("with_audit"), Some("true"));
        assert_eq!(
            get("filters"),
            Some(r#"{"phase":{"eq":"PHASE_TYPE_COMPLETE"},"trashed":{"eq":false}}"#)
        );
        assert_eq!(get("parent_id"), None, "根目录不应带 parent_id");
        assert_eq!(get("page_token"), None);
        assert!(!get("filters").unwrap_or_default().contains("kind"));

        let child = list_query(Some("folder-1"), Some("page-2"));
        assert!(child.contains(&("parent_id".to_owned(), "folder-1".to_owned())));
        assert!(child.contains(&("page_token".to_owned(), "page-2".to_owned())));
    }

    #[test]
    fn file_list_parses_lenient_fields() {
        let json = r#"{
            "kind": "drive#fileList",
            "files": [
                {"id":"f1","name":"电影","kind":"drive#folder","size":"0",
                 "modified_time":"2026-10-20T12:34:56.789+08:00"},
                {"id":"f2","name":"a.mkv","kind":"drive#file","size":"1099511627776",
                 "modified_time":null,"md5_checksum":"ABC",
                 "audit":{"status":"STATUS_OK"}},
                {"id":"f3","name":"bad.mkv","kind":"drive#file","size":"12",
                 "modified_time":"","audit":{"status":"STATUS_BANNED"}}
            ],
            "next_page_token": "tok"
        }"#;
        let parsed: FileListResponse = serde_json::from_str(json).expect("列表应可解析");
        assert_eq!(parsed.files.len(), 3);
        let entries: Vec<FileEntry> = parsed.files.into_iter().map(FileDto::into_entry).collect();

        assert_eq!(entries[0].kind, EntryKind::Folder);
        assert_eq!(
            entries[0].modified_at.map(|t| t.timestamp()),
            Some(1_792_470_896),
            "带毫秒与时区的时间应正确换算"
        );

        assert_eq!(entries[1].kind, EntryKind::File);
        assert_eq!(entries[1].size, 1_099_511_627_776, "字符串 size 应可解析");
        assert_eq!(entries[1].md5.as_deref(), Some("ABC"));
        assert!(!entries[1].audit_flagged);

        assert_eq!(entries[2].modified_at, None, "空时间应为 None");
        assert!(entries[2].audit_flagged, "非 STATUS_OK 的审核状态应标记");
    }
}
