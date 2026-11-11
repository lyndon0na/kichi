//! 网盘侧接口：容量 / 会员 / 当前用户 / 文件列表。
//!
//! 每个请求都带 `x-captcha-token`；官方以 `captcha_invalid` 拒绝时，
//! 作废并重取一次再试（rclone 同款兜底）。
//!
//! 按端点拆分（守行数软上限）：账号与配额在 [`account`]，文件 / 回收站列表
//! 与条目 DTO 在 [`files`]，媒体类型判断在 [`media`]；本文件保留客户端本身、
//! 统一请求入口、头像 / 缩略图下载与跨模块常量。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Deserialize;

use crate::captcha::CaptchaSource;
use crate::http::{Body, HttpClient, PublicImagePolicy};
use crate::{Config, Error};

mod account;
mod files;
mod media;
#[cfg(test)]
mod tests;

pub(crate) use files::{audit_flagged, lenient_time, AuditDto, FileDto};
pub use media::{is_image_name, is_media_name, is_video_name};
pub(crate) use media::{is_media, is_video, AUDIO_EXTENSIONS, IMAGE_EXTENSIONS, VIDEO_EXTENSIONS};

/// 单页最多返回的条目数（rclone 的 `ListLimit`）。
///
/// 注意：官方对全盘列举**固定每页 400 条**（2026-10-26 实测），给 500 也不会变。
pub(crate) const LIST_LIMIT: u32 = 500;
/// 文件列表只取已完成、未进回收站的条目。
pub(crate) const LIST_FILTERS: &str =
    r#"{"phase":{"eq":"PHASE_TYPE_COMPLETE"},"trashed":{"eq":false}}"#;
/// 已加星标条目：星标是系统标签，不是列表项上的 `starred` 布尔字段。
pub(crate) const STARRED_FILTERS: &str =
    r#"{"phase":{"eq":"PHASE_TYPE_COMPLETE"},"system_tag":{"in":"STAR"},"trashed":{"eq":false}}"#;
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
    /// 没有总时限，复用不会砍断大文件）。
    ///
    /// reqwest 的连接池与 TLS session 都在 client 里，每个文件重建一次等于
    /// 每文件重做握手 + 再起一条专属运行时线程。
    ///
    /// `pub(crate)`：`media.rs` 的 `impl DriveClient` 在另一个模块里取它。
    pub(crate) download: Mutex<Option<(Duration, Arc<reqwest::blocking::Client>)>>,
    /// 上传专用 client：同一个 `DriveClient` 的所有文件共用连接池。
    ///
    /// 上传请求按单次 S3 请求设置总时限，和下载的「空闲读超时」语义不同，
    /// 因此单独缓存；清理请求仍在 `upload.rs` 里按 5 秒时限临时构建。
    pub(crate) upload: Mutex<Option<Arc<reqwest::blocking::Client>>>,
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
            upload: Mutex::new(None),
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

    /// 同 [`DriveClient::request`]，但允许成功响应为空，返回 `None`。
    pub(crate) fn request_optional_json<R: serde::de::DeserializeOwned>(
        &self,
        method: reqwest::Method,
        url: &str,
        query: &[(String, String)],
        body: Body<'_>,
    ) -> Result<Option<R>, Error> {
        let action = action_for(method.as_str(), url);
        let token = self.captcha.acquire(&action, None, Some(&self.user_id))?;
        let access = Some(self.access_token.as_ref());
        let first = self.http.call_with_query_optional_json(
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
                self.http.call_with_query_optional_json(
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
}

pub(super) fn action_for(method: &str, url: &str) -> String {
    let path = url
        .split("://")
        .nth(1)
        .and_then(|rest| rest.find('/').map(|idx| &rest[idx..]))
        .unwrap_or(url);
    let path = path.split('?').next().unwrap_or(path);
    format!("{method}:{path}")
}
