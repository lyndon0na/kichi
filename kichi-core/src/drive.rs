//! 网盘侧接口：容量 / 会员 / 当前用户 / 文件列表。
//!
//! 每个请求都带 `x-captcha-token`；官方以 `captcha_invalid` 拒绝时，
//! 作废并重取一次再试（rclone 同款兜底）。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer};

use crate::captcha::CaptchaSource;
use crate::http::{Body, HttpClient, PublicImagePolicy};
use crate::types::{EntryKind, FileEntry, Quota, TrashEntry};
use crate::{Config, Error};

/// 单页最多返回的条目数（rclone 的 `ListLimit`）。
///
/// 注意：官方对全盘列举**固定每页 400 条**（2026-10-26 实测），给 500 也不会变。
pub(crate) const LIST_LIMIT: u32 = 500;
/// 文件列表只取已完成、未进回收站的条目。
pub(crate) const LIST_FILTERS: &str =
    r#"{"phase":{"eq":"PHASE_TYPE_COMPLETE"},"trashed":{"eq":false}}"#;
/// 回收站不按完成阶段收窄；`parent_id=*` 才能看到所有目录里删掉的条目。
pub(crate) const TRASH_FILTERS: &str = r#"{"trashed":{"eq":true}}"#;
pub(crate) const TRASH_PARENT_ID: &str = "*";

/// 已登录的网盘客户端（内部令牌由调用方的 `Session` 提供）。
pub struct DriveClient {
    config: Config,
    http: HttpClient,
    captcha: CaptchaSource,
    access_token: Arc<str>,
    user_id: String,
    /// 下载 client：按空闲读超时缓存（`.timeout` 是「响应头 ≤ D + 每次 read ≤ D」，
    /// 没有总时限，复用不会砍断大文件——见 `docs/pitfalls.md` 第 14 条）。
    ///
    /// reqwest 的连接池与 TLS session 都在 client 里，每个文件重建一次等于
    /// 每文件重做握手 + 再起一条专属运行时线程（`docs/pitfalls.md` 第 17 条）。
    ///
    /// `pub(crate)`：`media.rs` 的 `impl DriveClient` 在另一个模块里取它。
    pub(crate) download: Mutex<Option<(Duration, Arc<reqwest::blocking::Client>)>>,
}

/// 当前用户摘要。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct UserInfo {
    /// 官方字段名是 `sub`；旧响应若直接给 `id` 也兼容。
    #[serde(default, rename = "sub", alias = "id")]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub email: String,
    /// 头像地址。只留在 Rust 侧，由 `Session` 下载到本地缓存后 QML 才可读。
    #[serde(default)]
    pub picture: String,
}

#[derive(Deserialize)]
struct AboutResponse {
    #[serde(default)]
    quota: Option<AboutQuota>,
    #[serde(default)]
    quotas: Option<AboutQuotas>,
}

#[derive(Deserialize, Default)]
struct AboutQuotas {
    #[serde(default)]
    cloud_download: Option<AboutQuota>,
}

#[derive(Deserialize)]
struct AboutQuota {
    #[serde(default, deserialize_with = "lenient_u64")]
    usage: u64,
    #[serde(default, deserialize_with = "lenient_signed")]
    limit: i64,
    #[serde(default)]
    is_unlimited: bool,
}

/// 官方把额度编码成字符串，且「不限量」用 `"-1"` 表示。
fn lenient_signed<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error as _;
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Number(number) => Ok(number.as_i64().unwrap_or(0)),
        serde_json::Value::String(text) => text.parse::<i64>().map_err(D::Error::custom),
        _ => Ok(0),
    }
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
            download: Mutex::new(None),
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

    /// 同 [`DriveClient::request`]，但带 JSON 请求体、且不解析响应。
    ///
    /// 写操作（改名 / 新建 / 移动 / 复制）成功时官方不一定回 JSON，
    /// 解析反而会把成功当失败。
    pub(crate) fn request_no_response_body(
        &self,
        method: reqwest::Method,
        url: &str,
        query: &[(String, String)],
        body: &serde_json::Value,
    ) -> Result<(), Error> {
        let action = action_for(method.as_str(), url);
        let token = self.captcha.acquire(&action, None, Some(&self.user_id))?;
        let access = Some(self.access_token.as_ref());
        let first = self.http.call_with_query_no_response(
            method.clone(),
            url,
            query,
            Body::Json(body),
            access,
            Some(token.token.as_str()).filter(|t| !t.is_empty()),
        );
        match first {
            Err(err) if err.is_captcha() => {
                self.captcha.invalidate();
                let retry = self.captcha.acquire(&action, None, Some(&self.user_id))?;
                self.http.call_with_query_no_response(
                    method,
                    url,
                    query,
                    Body::Json(body),
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
        // 离线下载额度实测存在（quotas.cloud_download）；`limit` 为 "-1" 表示不限量，
        // 因此这里用 lenient 的字符串解析，不能按无符号数直接读
        let download = response.quotas.and_then(|quotas| quotas.cloud_download);
        let (download_usage, download_limit) = match download {
            Some(entry) => (
                Some(entry.usage),
                if entry.is_unlimited || entry.limit < 0 {
                    Some(None)
                } else {
                    Some(Some(entry.limit as u64))
                },
            ),
            None => (None, None),
        };
        Ok(Quota {
            usage: quota.usage,
            limit: (!quota.is_unlimited && quota.limit > 0).then_some(quota.limit as u64),
            vip,
            cloud_download_usage: download_usage,
            cloud_download_limit: download_limit,
        })
    }

    /// 排障用：`about` 的原始响应（只给集成测试打印字段名，不对外暴露）。
    #[doc(hidden)]
    pub fn about_raw_for_probe(&self) -> Result<serde_json::Value, Error> {
        self.get(&self.config.about_url())
    }

    /// 会员状态（失败不致命，`about` 里按 false 处理）。
    pub fn is_vip(&self) -> Result<bool, Error> {
        let vip: VipResponse = self.get(&self.config.vip_url())?;
        Ok(vip.data.is_some_and(|data| {
            data.status.eq_ignore_ascii_case("ok") && !data.vip_type.eq_ignore_ascii_case("novip")
        }))
    }

    /// 当前用户（昵称 / 头像 / 邮箱用来在界面上显示身份）。
    pub fn me(&self) -> Result<UserInfo, Error> {
        self.get(&self.config.user_me_url())
    }

    /// 下载当前账号头像。
    ///
    /// 头像 URL 与缩略图一样按不可信外部输入处理：只允许 HTTPS +
    /// `mypikpak.com`，或经用户批准的 Google 资料图主机
    /// `lh3.googleusercontent.com`；不跟随重定向，最多 2 MiB。
    pub fn fetch_avatar(&self, url: &str) -> Result<Vec<u8>, Error> {
        self.http
            .fetch_public_bytes(url, 2 * 1024 * 1024, PublicImagePolicy::Avatar)
    }

    /// 下载列表下发的小缩略图。
    ///
    /// `thumbnail_link` 指向 PikPak 缩略图 CDN，不带签名或查询参数。这里仍把它
    /// 当作不可信外部输入：只允许 HTTPS + `mypikpak.com` 主机，并限制体积。
    pub fn fetch_thumbnail(&self, url: &str) -> Result<Vec<u8>, Error> {
        self.http
            .fetch_public_bytes(url, 4 * 1024 * 1024, PublicImagePolicy::PikPakOnly)
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

    /// 读取单个条目的完整文件信息（只读）。
    ///
    /// 资料库事件通常带 `reference_resource`；只有资源缺失或不足以识别
    /// 文件 / 文件夹时才回退到这里，避免为整页事件逐条发详情请求。
    pub fn file_entry(&self, id: &str) -> Result<FileEntry, Error> {
        let id = id.trim();
        if id.is_empty() {
            return Err(Error::InvalidInput("条目 id 为空".into()));
        }
        let dto: FileDto = self.get(&self.config.file_url(id))?;
        Ok(dto.into_entry())
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

/// 回收站查询参数。单独成函数，测试能直接锁住这两个容易漏的条件。
fn trash_query(page_token: Option<&str>) -> Vec<(String, String)> {
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
struct FileListResponse {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    files: Vec<FileDto>,
    #[serde(default)]
    next_page_token: String,
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
    /// 所在文件夹 id：全盘列举时靠它还原真实路径；根目录的条目为空串
    #[serde(default)]
    parent_id: String,
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

    /// 事件里的参考资源是否足以直接构造列表行。
    ///
    /// 时间与缩略图可能本来就不下发，不算「不完整」；身份字段缺失、
    /// 类型未知或与事件 `file_id` 不一致时才必须补一次详情请求。
    pub(crate) fn is_complete_event_resource(&self, expected_id: &str) -> bool {
        let id = self.id.trim();
        !id.is_empty()
            && id == expected_id.trim()
            && !self.name.trim().is_empty()
            && EntryKind::from_api_kind(&self.kind).is_some()
    }

    pub(crate) fn into_complete_event_entry(self, expected_id: &str) -> Option<FileEntry> {
        self.is_complete_event_resource(expected_id)
            .then(|| self.into_entry())
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
            parent_id: self.parent_id.trim().to_owned(),
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

    /// 转换成回收站条目：保留原父目录与删除时间。
    pub(crate) fn into_trash_entry(self) -> TrashEntry {
        let parent_id = self.parent_id.trim().to_owned();
        let deleted_at = self.delete_time;
        TrashEntry {
            entry: self.into_entry(),
            parent_id,
            deleted_at,
        }
    }

    /// 转换成搜索命中：条目 + 所在文件夹 id（全盘列举用）。
    pub(crate) fn into_hit(self) -> crate::search::FileHit {
        let parent_id = self.parent_id.trim().to_owned();
        crate::search::FileHit {
            entry: self.into_entry(),
            parent_id,
        }
    }

    /// 单个条目自身：名字 + 父目录 id（拼「所在文件夹」路径用）。
    pub(crate) fn into_name_and_parent(self) -> (String, String) {
        (self.name, self.parent_id.trim().to_owned())
    }
}

/// 视频扩展名表（判断播放入口、自动加载字幕用）。
pub(crate) const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "mkv", "avi", "mov", "webm", "ts", "m2ts", "m4v", "flv", "wmv", "rmvb", "rm", "mpg",
    "mpeg", "3gp", "ogv",
];

/// 音频扩展名表：能播放，但不尝试挂外挂字幕。
pub(crate) const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "wav", "m4a", "ogg", "opus", "aac", "ape", "wma",
];

/// 图片扩展名表：只有图片和视频的列表缩略图会进入本地缓存。
pub(crate) const IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "bmp", "avif", "heic", "heif", "tif", "tiff",
];

fn extension_matches(
    name: &str,
    category: &str,
    mime: &str,
    extension: &str,
    kind: &str,
    allowed: &[&str],
) -> bool {
    let from_extension = |text: &str| {
        let ext = text.trim().trim_start_matches('.').to_ascii_lowercase();
        allowed.contains(&ext.as_str())
    };
    if !extension.is_empty() && from_extension(extension) {
        return true;
    }
    if name
        .rsplit_once('.')
        .is_some_and(|(_, ext)| from_extension(ext))
    {
        return true;
    }
    category.trim().eq_ignore_ascii_case(kind)
        || mime
            .trim()
            .to_ascii_lowercase()
            .starts_with(&format!("{}/", kind.to_ascii_lowercase()))
}

pub(crate) fn is_video(name: &str, category: &str, mime: &str, extension: &str) -> bool {
    extension_matches(name, category, mime, extension, "VIDEO", VIDEO_EXTENSIONS)
}

fn is_audio(name: &str, category: &str, mime: &str, extension: &str) -> bool {
    extension_matches(name, category, mime, extension, "AUDIO", AUDIO_EXTENSIONS)
}

fn is_image(name: &str, category: &str, mime: &str, extension: &str) -> bool {
    extension_matches(name, category, mime, extension, "IMAGE", IMAGE_EXTENSIONS)
}

/// 是否按媒体处理（决定是否显示播放入口）。
///
/// **实测（2026-10-26）**：文件列表接口在不带 `kind` 过滤时，
/// `file_category` / `mime_type` / `file_extension` 全是空串（带 `kind`
/// 过滤才下发），因此不能只信官方分类，必须按扩展名兜底，否则视频文件
/// 在界面上没有播放按钮。官方分类可用时优先采信。
pub(crate) fn is_media(name: &str, category: &str, mime: &str, extension: &str) -> bool {
    is_video(name, category, mime, extension) || is_audio(name, category, mime, extension)
}

/// RFC3339 时间；官方可能给 `null`、空串或无法解析的值，一律按“没有时间”处理。
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
        assert!(is_video("Screencast.webm", "", "", ""));
        assert!(!is_video("song.flac", "", "", ""));
        assert!(is_image("cover.JPG", "", "", ""));
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
    fn user_info_parses_nickname_avatar_and_email() {
        let raw = r#"{
            "sub":"user-1",
            "name":"夏日用户",
            "picture":"https://user.mypikpak.com/avatar/1",
            "email":"mail@example.com"
        }"#;
        let user: UserInfo = serde_json::from_str(raw).expect("用户信息应可解析");
        assert_eq!(user.id, "user-1");
        assert_eq!(user.name, "夏日用户");
        assert_eq!(user.picture, "https://user.mypikpak.com/avatar/1");
        assert_eq!(user.email, "mail@example.com");
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
    fn trash_query_covers_all_directories_and_excludes_active_files() {
        let query = trash_query(Some("next"));
        let get = |key: &str| {
            query
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(get("parent_id"), Some("*"), "必须列全账号，不只根目录");
        assert_eq!(
            get("filters"),
            Some(r#"{"trashed":{"eq":true}}"#),
            "回收站只按 trashed 过滤，不要求 phase=complete"
        );
        assert_eq!(get("page_token"), Some("next"));
    }

    #[test]
    fn file_list_parses_lenient_fields() {
        let json = r#"{
            "kind": "drive#fileList",
            "files": [
                {"id":"f1","name":"电影","kind":"drive#folder","size":"0",
                 "created_time":"2026-10-20T12:00:00+08:00","starred":true,
                 "modified_time":"2026-10-20T12:34:56.789+08:00"},
                {"id":"f2","name":"a.mkv","kind":"drive#file","size":"1099511627776",
                 "modified_time":null,"md5_checksum":"ABC",
                 "parent_id":"folder-7","tags":[{"name":"STAR"}],
                 "audit":{"status":"STATUS_OK"},
                 "thumbnail_link":"https://sg-thumbnail-drive.mypikpak.com/v0/x/240/720"},
                {"id":"f3","name":"bad.mkv","kind":"drive#file","size":"12",
                 "modified_time":"","tags":null,"audit":{"status":"STATUS_BANNED"}}
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
        assert_eq!(
            entries[0].created_at.map(|t| t.to_rfc3339()),
            Some("2026-10-20T04:00:00+00:00".to_owned()),
            "创建时间也应宽松解析"
        );
        assert!(entries[0].starred, "显式 starred 应识别");

        assert_eq!(entries[1].kind, EntryKind::File);
        assert_eq!(entries[1].size, 1_099_511_627_776, "字符串 size 应可解析");
        assert_eq!(entries[1].parent_id, "folder-7");
        assert!(entries[1].starred, "tags 里的 STAR 也应识别为已加星标");
        assert_eq!(entries[1].md5.as_deref(), Some("ABC"));
        assert!(!entries[1].audit_flagged);
        assert!(entries[1].media);
        assert!(entries[1].video);
        assert_eq!(
            entries[1].thumbnail_url,
            "https://sg-thumbnail-drive.mypikpak.com/v0/x/240/720"
        );
        assert!(entries[0].thumbnail_url.is_empty(), "文件夹不请求缩略图");

        assert_eq!(entries[2].modified_at, None, "空时间应为 None");
        assert!(entries[2].audit_flagged, "非 STATUS_OK 的审核状态应标记");
    }

    #[test]
    fn search_hit_keeps_the_parent_folder() {
        // 全盘列举的条目带 parent_id，拼「所在文件夹」真实路径全靠它
        let json = r#"{"files":[
            {"id":"f1","name":"第 1 集.mkv","kind":"drive#file","parent_id":"season-3"},
            {"id":"f2","name":"根目录文件.mkv","kind":"drive#file"}
        ]}"#;
        let parsed: FileListResponse = serde_json::from_str(json).expect("应可解析");
        let hits: Vec<crate::search::FileHit> =
            parsed.files.into_iter().map(FileDto::into_hit).collect();
        assert_eq!(hits[0].parent_id, "season-3");
        assert_eq!(hits[0].entry.name, "第 1 集.mkv");
        assert_eq!(hits[1].parent_id, "", "官方没给 parent_id 按根目录处理");
    }

    #[test]
    fn trash_entry_keeps_parent_and_delete_time() {
        let json = r#"{"files":[
            {"id":"f1","name":"旧文件.mkv","kind":"drive#file","size":"42",
             "parent_id":"folder-7","delete_time":"2026-10-28T09:10:11+08:00"},
            {"id":"f2","name":"旧目录","kind":"drive#folder","delete_time":null}
        ]}"#;
        let parsed: FileListResponse = serde_json::from_str(json).expect("回收站应可解析");
        let trash: Vec<TrashEntry> = parsed
            .files
            .into_iter()
            .map(FileDto::into_trash_entry)
            .collect();
        assert_eq!(trash[0].entry.id, "f1");
        assert_eq!(trash[0].parent_id, "folder-7");
        assert_eq!(
            trash[0].deleted_at.map(|time| time.timestamp()),
            Some(1_793_149_811)
        );
        assert_eq!(trash[1].parent_id, "", "缺 parent_id 按根目录处理");
        assert_eq!(trash[1].deleted_at, None, "空删除时间应可解析");
    }

    /// 用 2026-10-26 真实账号抓到的 `about` 响应结构做样本。
    #[test]
    fn about_parses_cloud_download_quota() {
        let raw = r#"{
            "kind": "drive#about",
            "quota": {"kind": "drive#quota", "limit": "10995116277760", "usage": "5230119883333",
                      "usage_in_trash": "0", "is_unlimited": false},
            "quotas": {"cloud_download": {"kind": "", "limit": "-1", "usage": "0",
                       "usage_in_trash": "0", "is_unlimited": true}},
            "user_type": 1
        }"#;
        let response: AboutResponse = serde_json::from_str(raw).expect("样本应能解析");
        let download = response
            .quotas
            .and_then(|quotas| quotas.cloud_download)
            .expect("cloud_download 应存在");
        assert_eq!(download.usage, 0);
        assert!(download.is_unlimited, "limit=-1 表示不限量");
        assert_eq!(download.limit, -1, "-1 必须被宽容解析成负数而不是报错");
    }

    #[test]
    fn about_without_quotas_still_parses() {
        let raw = r#"{"quota": {"limit": "100", "usage": "50"}}"#;
        let response: AboutResponse = serde_json::from_str(raw).expect("缺 quotas 也应能解析");
        assert!(response.quotas.is_none());
        assert_eq!(response.quota.map(|q| q.usage), Some(50));
    }
}
