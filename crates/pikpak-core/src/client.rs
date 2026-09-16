use std::io::SeekFrom;
use std::path::Path as StdPath;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::sync::Mutex;

use crate::captcha::{build_user_agent, captcha_sign, now_ms};
use crate::consts::*;
use crate::download::{part_path, DownloadLink, MediaVariant};
use crate::error::{
    api_error, ApiErrorBody, Error, ERROR_CODE_CAPTCHA_INVALID, ERROR_CODE_TOKEN_EXPIRED,
    ERROR_INVALID_ACCOUNT,
};
use crate::session::Session;
use crate::types::*;
use crate::upload::{self, OssContext, UploadTicket};

/// 写入缓冲的落盘阈值: 每攒够这么多字节 flush 一次磁盘。
const FLUSH_INTERVAL: u64 = 8 * 1024 * 1024;

const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
    (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36";

/// OSS 请求使用的 User-Agent(沿用可用的阿里云 SDK 风格)。
const OSS_UA: &str = "aliyun-sdk-android/2.9.5";

#[derive(Default)]
struct Auth {
    access_token: String,
    refresh_token: String,
    user_id: String,
    username: String,
    captcha_token: Option<String>,
}

pub type TokenSaver = Arc<dyn Fn(&Session) + Send + Sync>;

pub struct PikPakClient {
    http: reqwest::Client,
    device_id: String,
    auth: Mutex<Auth>,
    /// 续期单飞: 保证同一时刻只有一个请求在刷新 token, 避免并发刷新与重复写盘。
    refresh_lock: Mutex<()>,
    on_tokens: Option<TokenSaver>,
}

impl PikPakClient {
    pub fn new(device_id: String) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(BROWSER_UA)
            .build()
            .expect("failed to build http client");
        Self {
            http,
            device_id,
            auth: Mutex::new(Auth::default()),
            refresh_lock: Mutex::new(()),
            on_tokens: None,
        }
    }

    pub fn set_token_saver(&mut self, saver: TokenSaver) {
        self.on_tokens = Some(saver);
    }

    pub async fn set_session(&self, session: &Session) {
        let mut a = self.auth.lock().await;
        a.access_token = session.access_token.clone();
        a.refresh_token = session.refresh_token.clone();
        a.user_id = session.user_id.clone();
        a.username = session.username.clone();
    }

    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    async fn current_auth(&self) -> (String, String, String, Option<String>) {
        let a = self.auth.lock().await;
        (
            a.access_token.clone(),
            a.refresh_token.clone(),
            a.user_id.clone(),
            a.captcha_token.clone(),
        )
    }

    fn save_tokens(&self, access: &str, refresh: &str, user_id: &str, username: &str) {
        if let Some(saver) = &self.on_tokens {
            saver(&Session {
                device_id: self.device_id.clone(),
                access_token: access.to_string(),
                refresh_token: refresh.to_string(),
                user_id: user_id.to_string(),
                username: username.to_string(),
            });
        }
    }

    async fn update_tokens(&self, access: &str, refresh: &str, user_id: &str) {
        let username = {
            let mut a = self.auth.lock().await;
            a.access_token = access.to_string();
            a.refresh_token = refresh.to_string();
            a.user_id = user_id.to_string();
            a.username.clone()
        };
        self.save_tokens(access, refresh, user_id, &username);
    }

    fn ua(&self, has_captcha: bool) -> &'static str {
        if has_captcha {
            "android"
        } else {
            BROWSER_UA
        }
    }

    /// 核心请求入口: 附带鉴权, 遇到 error_code=16 自动刷新并重试一次。
    async fn request(
        &self,
        method: reqwest::Method,
        url: &str,
        body: Option<&Value>,
        query: &[(&str, String)],
    ) -> Result<Value, Error> {
        self.request_inner(method, url, body, query, true).await
    }

    /// recover=true 时: code16 自动刷新 token、code9 自动重新初始化 captcha。
    async fn request_inner(
        &self,
        method: reqwest::Method,
        url: &str,
        body: Option<&Value>,
        query: &[(&str, String)],
        recover: bool,
    ) -> Result<Value, Error> {
        let mut attempt = 0;
        loop {
            let (token, _, user_id, captcha) = self.current_auth().await;
            let mut req = self
                .http
                .request(method.clone(), url)
                .header("Content-Type", "application/json; charset=utf-8")
                .header("X-Device-Id", &self.device_id);

            if !token.is_empty() {
                req = req.bearer_auth(&token);
            }
            let ua = self.ua(captcha.is_some());
            if ua == "android" {
                let full = build_user_agent(&self.device_id, &user_id);
                req = req.header("User-Agent", full);
            } else {
                req = req.header("User-Agent", ua);
            }
            if let Some(captcha) = &captcha {
                if !captcha.is_empty() {
                    req = req.header("X-Captcha-Token", captcha);
                }
            }
            if !query.is_empty() {
                req = req.query(query);
            }
            if let Some(b) = body {
                req = req.json(b);
            }

            let resp = req.send().await?;
            let status = resp.status();
            let bytes = resp.bytes().await?;

            let value: Value = if bytes.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&bytes).unwrap_or(Value::Null)
            };

            let err_body: ApiErrorBody = serde_json::from_slice(&bytes).unwrap_or_default();

            if !status.is_success() && err_body.error_code == 0 && err_body.error.is_empty() {
                let text = String::from_utf8_lossy(&bytes).to_string();
                tracing::debug!("HTTP {}: {}", status.as_u16(), truncate(&text, 200));
                return Err(Error::Msg(format!(
                    "HTTP {}: {}",
                    status.as_u16(),
                    truncate(&text, 300)
                )));
            }

            if err_body.error_code != 0 {
                if recover
                    && err_body.error_code == ERROR_CODE_TOKEN_EXPIRED
                    && attempt == 0
                    && !token.is_empty()
                {
                    tracing::info!("access token 过期, 正在自动续期");
                    self.refresh_token().await?;
                    attempt += 1;
                    continue;
                }
                // captcha token 失效/不匹配: 按本次请求的 action 重新初始化后再试。
                if recover && err_body.error_code == ERROR_CODE_CAPTCHA_INVALID && attempt == 0 {
                    let action = request_action(&method, url);
                    tracing::debug!("captcha 失效, 按 action={action} 重新初始化");
                    if self.captcha_init(&action, None).await.is_ok() {
                        attempt += 1;
                        continue;
                    }
                }
                if err_body.error == ERROR_INVALID_ACCOUNT {
                    return Err(Error::msg("账号或密码错误"));
                }
                tracing::debug!(
                    "API 错误: code={} error={} ({})",
                    err_body.error_code,
                    err_body.error,
                    err_body.error_description
                );
                return Err(api_error(&err_body));
            }
            if !err_body.error.is_empty() {
                return Err(api_error(&err_body));
            }

            return Ok(value);
        }
    }

    async fn post(&self, url: &str, body: &Value) -> Result<Value, Error> {
        self.request(reqwest::Method::POST, url, Some(body), &[])
            .await
    }

    async fn patch(&self, url: &str, body: &Value) -> Result<Value, Error> {
        self.request(reqwest::Method::PATCH, url, Some(body), &[])
            .await
    }

    async fn delete(&self, url: &str, query: &[(&str, String)]) -> Result<Value, Error> {
        self.request(reqwest::Method::DELETE, url, None, query)
            .await
    }

    async fn get(&self, url: &str, query: &[(&str, String)]) -> Result<Value, Error> {
        self.request(reqwest::Method::GET, url, None, query).await
    }

    /// 登录(账号密码), 需要先走 captcha init。
    pub async fn login(&self, username: &str, password: &str) -> Result<Session, Error> {
        let signin_url = format!("{USER_HOST}/v1/auth/signin");
        let mut meta = serde_json::Map::new();
        if username.contains('@') {
            meta.insert("email".into(), username.into());
        } else if username.chars().all(|c| c.is_ascii_digit()) {
            meta.insert("phone_number".into(), username.into());
        } else {
            meta.insert("username".into(), username.into());
        }

        let captcha = self
            .captcha_init(&format!("POST:{signin_url}"), Some(json!(meta)))
            .await?;

        let body = json!({
            "client_id": CLIENT_ID,
            "client_secret": CLIENT_SECRET,
            "password": password,
            "username": username,
            "captcha_token": captcha,
        });

        let resp = self
            .request(reqwest::Method::POST, &signin_url, Some(&body), &[])
            .await?;

        let token: TokenResponse = serde_json::from_value(resp)
            .map_err(|e| Error::Msg(format!("登录响应解析失败: {e}")))?;
        let user_id = token.sub.clone().unwrap_or_default();
        // 先写入账号名, 使续期/持久化时不会丢失。
        self.auth.lock().await.username = username.to_string();
        self.update_tokens(&token.access_token, &token.refresh_token, &user_id)
            .await;

        Ok(Session {
            device_id: self.device_id.clone(),
            access_token: token.access_token,
            refresh_token: token.refresh_token,
            user_id,
            username: username.to_string(),
        })
    }

    /// 用已有会话恢复登录(access token 过期时内部自动刷新)。
    pub async fn restore(&self, session: &Session) -> Result<(), Error> {
        {
            let mut a = self.auth.lock().await;
            a.access_token = session.access_token.clone();
            a.refresh_token = session.refresh_token.clone();
            a.user_id = session.user_id.clone();
            a.username = session.username.clone();
        }
        self.save_tokens(
            &session.access_token,
            &session.refresh_token,
            &session.user_id,
            &session.username,
        );
        Ok(())
    }

    pub async fn refresh_token(&self) -> Result<(), Error> {
        // 续期单飞: 进入时记录当前 access token, 加锁后若已被其它请求刷新则直接复用,
        // 避免并发请求同时刷新并重复写盘。
        let seen = self.current_auth().await.0;
        let _guard = self.refresh_lock.lock().await;
        let (token, refresh, _, _) = self.current_auth().await;
        if token != seen {
            return Ok(());
        }
        if refresh.is_empty() {
            return Err(Error::AuthExpired("缺少 refresh token".into()));
        }
        let url = format!("{USER_HOST}/v1/auth/token");
        let body = json!({
            "client_id": CLIENT_ID,
            "refresh_token": refresh,
            "grant_type": "refresh_token",
        });
        let resp = self
            .http
            .post(&url)
            .header("Content-Type", "application/json; charset=utf-8")
            .header("User-Agent", BROWSER_UA)
            .json(&body)
            .send()
            .await?;
        let bytes = resp.bytes().await?;
        let err_body: ApiErrorBody = serde_json::from_slice(&bytes).unwrap_or_default();
        if err_body.error_code != 0 || !err_body.error.is_empty() {
            tracing::warn!(
                "续期失败(code={}): {}",
                err_body.error_code,
                if err_body.error_description.is_empty() {
                    &err_body.error
                } else {
                    &err_body.error_description
                }
            );
            // refresh token 过期/被吊销: 归为登录态失效, 交由上层要求重新登录。
            return Err(Error::AuthExpired(if err_body.error_description.is_empty() {
                err_body.error
            } else {
                err_body.error_description
            }));
        }
        let token: TokenResponse = serde_json::from_slice(&bytes)?;
        let user_id = token.sub.clone().unwrap_or_default();
        self.update_tokens(&token.access_token, &token.refresh_token, &user_id)
            .await;
        tracing::info!("登录态续期成功");
        Ok(())
    }

    /// captcha init: 拿到 captcha_token。meta 为空时自动生成 captcha_sign。
    /// 注意: 走裸请求, 不经过自动恢复逻辑, 避免递归。
    pub async fn captcha_init(&self, action: &str, meta: Option<Value>) -> Result<String, Error> {
        let url = format!("{USER_HOST}/v1/shield/captcha/init");
        let ts = now_ms();
        let (_, _, user_id, _) = self.current_auth().await;

        let meta = meta.unwrap_or_else(|| {
            json!({
                "captcha_sign": captcha_sign(&self.device_id, ts),
                "client_version": CLIENT_VERSION,
                "package_name": PACKAGE_NAME,
                "user_id": user_id,
                "timestamp": ts.to_string(),
            })
        });

        let body = json!({
            "client_id": CLIENT_ID,
            "action": action,
            "device_id": self.device_id,
            "meta": meta,
        });

        let mut req = self
            .http
            .post(&url)
            .header("Content-Type", "application/json; charset=utf-8")
            .header("X-Device-Id", &self.device_id)
            .header("User-Agent", BROWSER_UA)
            .json(&body);

        let (token, _, _, _) = self.current_auth().await;
        if !token.is_empty() {
            req = req.bearer_auth(&token);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| Error::Msg(format!("captcha 初始化失败: {e}")))?;
        let bytes = resp.bytes().await?;
        let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);

        let out = value
            .get("captcha_token")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        if out.is_empty() {
            let msg = value
                .get("error_description")
                .and_then(|v| v.as_str())
                .unwrap_or("未获取到 captcha_token(可能需要通过网页验证)")
                .to_string();
            return Err(Error::msg(msg));
        }
        {
            let mut a = self.auth.lock().await;
            a.captcha_token = Some(out.clone());
        }
        Ok(out)
    }

    // ---------- 文件 ----------

    pub async fn file_list(
        &self,
        parent_id: Option<&str>,
        size: usize,
        next_page_token: Option<&str>,
    ) -> Result<FileList, Error> {
        let url = format!("{API_HOST}/drive/v1/files");
        let mut query: Vec<(&str, String)> = vec![
            ("thumbnail_size", "SIZE_MEDIUM".into()),
            ("limit", size.to_string()),
            ("with_audit", "true".into()),
            ("filters", default_file_filters().to_string()),
        ];
        if let Some(pid) = parent_id {
            if !pid.is_empty() {
                query.push(("parent_id", pid.to_string()));
            }
        }
        if let Some(tok) = next_page_token {
            query.push(("page_token", tok.to_string()));
        }
        let value = self.get(&url, &query).await?;
        let list: FileList = serde_json::from_value(value)?;
        Ok(list)
    }

    pub async fn create_folder(&self, name: &str, parent_id: Option<&str>) -> Result<Value, Error> {
        let url = format!("{API_HOST}/drive/v1/files");
        let mut body = serde_json::Map::new();
        body.insert("kind".into(), "drive#folder".into());
        body.insert("name".into(), name.into());
        if let Some(pid) = parent_id {
            if !pid.is_empty() {
                body.insert("parent_id".into(), pid.into());
            }
        }
        self.post(&url, &Value::Object(body)).await
    }

    pub async fn rename(&self, id: &str, new_name: &str) -> Result<Value, Error> {
        let url = format!("{API_HOST}/drive/v1/files/{id}");
        let body = json!({ "name": new_name });
        self.patch(&url, &body).await
    }

    pub async fn batch_trash(&self, ids: &[String]) -> Result<Value, Error> {
        let url = format!("{API_HOST}/drive/v1/files:batchTrash");
        self.post(&url, &json!({ "ids": ids })).await
    }

    /// 批量把文件/文件夹移动到目标目录(to_parent_id 为空表示移动到根目录)。
    pub async fn batch_move(
        &self,
        ids: &[String],
        to_parent_id: Option<&str>,
    ) -> Result<Value, Error> {
        let url = format!("{API_HOST}/drive/v1/files:batchMove");
        self.post(&url, &move_copy_body(ids, to_parent_id)).await
    }

    /// 批量复制文件/文件夹到目标目录。
    pub async fn batch_copy(
        &self,
        ids: &[String],
        to_parent_id: Option<&str>,
    ) -> Result<Value, Error> {
        let url = format!("{API_HOST}/drive/v1/files:batchCopy");
        self.post(&url, &move_copy_body(ids, to_parent_id)).await
    }

    // ---------- 离线下载 ----------

    pub async fn offline_create(
        &self,
        link: &str,
        name: Option<&str>,
        parent_id: Option<&str>,
    ) -> Result<Value, Error> {
        let url = format!("{API_HOST}/drive/v1/files");
        let mut body = serde_json::Map::new();
        body.insert("kind".into(), "drive#file".into());
        body.insert("upload_type".into(), "UPLOAD_TYPE_URL".into());
        body.insert("url".into(), json!({ "url": link }));
        match parent_id {
            Some(pid) if !pid.is_empty() => {
                body.insert("parent_id".into(), pid.into());
            }
            _ => {
                body.insert("folder_type".into(), "DOWNLOAD".into());
            }
        }
        if let Some(name) = name {
            if !name.is_empty() {
                body.insert("name".into(), name.into());
            }
        }
        self.post(&url, &Value::Object(body)).await
    }

    pub async fn offline_list_phase(
        &self,
        phase: &str,
        size: usize,
        next_page_token: Option<&str>,
    ) -> Result<Tasks, Error> {
        let url = format!("{API_HOST}/drive/v1/tasks");
        let filters = json!({ "phase": { "in": phase } });
        let mut query: Vec<(&str, String)> = vec![
            ("type", "offline".into()),
            ("thumbnail_size", "SIZE_SMALL".into()),
            ("limit", size.to_string()),
            ("filters", filters.to_string()),
            ("with", "reference_resource".into()),
        ];
        if let Some(tok) = next_page_token {
            query.push(("page_token", tok.to_string()));
        }
        let value = self.get(&url, &query).await?;
        Ok(Tasks::parse(value))
    }

    pub async fn offline_retry(&self, task_id: &str) -> Result<Value, Error> {
        let url = format!("{API_HOST}/drive/v1/task");
        let body = json!({
            "type": "offline",
            "create_type": "RETRY",
            "id": task_id,
        });
        self.post(&url, &body).await
    }

    pub async fn offline_delete(
        &self,
        task_ids: &[String],
        delete_files: bool,
    ) -> Result<(), Error> {
        let url = format!("{API_HOST}/drive/v1/tasks");
        let mut query: Vec<(&str, String)> = Vec::new();
        for id in task_ids {
            query.push(("task_ids", id.clone()));
        }
        query.push(("delete_files", delete_files.to_string()));
        self.delete(&url, &query).await?;
        Ok(())
    }

    // ---------- 账户 ----------

    pub async fn quota(&self) -> Result<Quota, Error> {
        let url = format!("{API_HOST}/drive/v1/about");
        let value = self.get(&url, &[]).await?;
        let resp: QuotaResponse = serde_json::from_value(value)?;
        Ok(resp.quota)
    }

    pub async fn logout(&self) {
        let mut a = self.auth.lock().await;
        a.access_token.clear();
        a.refresh_token.clear();
        a.captcha_token = None;
    }

    /// 清除当前 captcha token。captcha token 是一次性、按 action 签发的,
    /// 用完应立即释放, 避免残留 token 影响后续请求的 UA/请求头。
    pub async fn clear_captcha_token(&self) {
        let mut a = self.auth.lock().await;
        a.captcha_token = None;
    }

    // ---------- 本地下载 ----------

    /// 返回访问签名直链所需的最小请求头(User-Agent / X-Device-Id, 必要时附加 Bearer)。
    ///
    /// 先按与下载相同的策略探测: 不带 Bearer 发一个单字节 `Range` 请求, 若被拒
    /// (401/403) 再带 Bearer 重试一次, 取服务端接受的那一组。避免向本就免鉴权的
    /// 签名直链附加多余的 Authorization(部分 CDN 会因此重置连接、读到 0 字节)。
    pub async fn stream_headers(&self, url: &str) -> Vec<(String, String)> {
        let (token, _, user_id, _) = self.current_auth().await;
        let ua = build_user_agent(&self.device_id, &user_id);
        let mut headers = vec![
            ("User-Agent".to_string(), ua.clone()),
            ("X-Device-Id".to_string(), self.device_id.clone()),
        ];
        let accept = |s: reqwest::StatusCode| {
            s.is_success() || s == reqwest::StatusCode::PARTIAL_CONTENT
        };

        let first = self
            .http
            .get(url)
            .header("User-Agent", &ua)
            .header("X-Device-Id", &self.device_id)
            .header("Range", "bytes=0-0")
            .send()
            .await;
        if let Ok(resp) = first {
            let status = resp.status();
            drop(resp);
            if accept(status) {
                return headers;
            }
            if (status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN)
                && !token.is_empty()
            {
                let retry = self
                    .http
                    .get(url)
                    .header("User-Agent", &ua)
                    .header("X-Device-Id", &self.device_id)
                    .header("Range", "bytes=0-0")
                    .bearer_auth(&token)
                    .send()
                    .await;
                if let Ok(resp) = retry {
                    let status = resp.status();
                    drop(resp);
                    if accept(status) {
                        headers.push(("Authorization".to_string(), format!("Bearer {token}")));
                    }
                }
            }
        }
        headers
    }

    /// 拉取文件详情。需先对该 action 做 captcha init, 且 captcha token 用后即焚。
    /// 目标是文件夹时返回错误。
    async fn fetch_file_detail(&self, file_id: &str) -> Result<Value, Error> {
        let id = file_id.trim();
        if id.is_empty() {
            return Err(Error::msg("文件 ID 为空"));
        }
        // 与 request_action 推导保持一致: 详情请求的 action 形如 GET:/drive/v1/files/{id}。
        let action = format!("GET:/drive/v1/files/{id}");
        self.captcha_init(&action, None).await?;
        let url = format!("{API_HOST}/drive/v1/files/{id}");
        let detail = self
            .get(&url, &[("thumbnail_size", "SIZE_LARGE".to_string())])
            .await;
        // captcha token 用后即焚, 避免影响后续普通请求。
        self.clear_captcha_token().await;
        let detail = detail?;
        // 若目标是文件夹(离线任务可能指向整目录), 直接给出清晰提示。
        if detail
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .contains("folder")
        {
            return Err(Error::msg("该目标是一个文件夹, 暂只支持单个文件"));
        }
        Ok(detail)
    }

    /// 解析某个文件的下载直链。
    pub async fn file_download_link(&self, file_id: &str) -> Result<DownloadLink, Error> {
        let detail = self.fetch_file_detail(file_id).await?;
        DownloadLink::from_detail(file_id.trim(), &detail)
    }

    /// 解析某个媒体文件可用的清晰度列表(原画在前, 其余按分辨率降序)。
    /// 无转码流时回退为仅原画; 无任何可用直链时返回错误。
    pub async fn media_variants(&self, file_id: &str) -> Result<Vec<MediaVariant>, Error> {
        let detail = self.fetch_file_detail(file_id).await?;
        let mut variants = MediaVariant::from_detail(&detail);
        if variants.is_empty() {
            let dl = DownloadLink::from_detail(file_id.trim(), &detail)?;
            variants.push(MediaVariant {
                label: "原画".to_string(),
                url: dl.url,
                is_origin: true,
                height: 0,
            });
        }
        Ok(variants)
    }

    /// 把某个文件(已拿到直链)流式下载到 `dest`。
    ///
    /// 写盘采用 `<dest>.part` 临时文件, 成功后再原子改名; 若 `.part` 已存在则用
    /// `Range` 续传。`cancel` 置位时会在读取到下一个分片时中止(保留已下载的 part)。
    /// `on_progress` 每次读取分片后回调 `(总大小, 已下载)`; 总大小未知时为 0。
    pub async fn download_to<F>(
        &self,
        link: &DownloadLink,
        dest: &StdPath,
        cancel: Option<Arc<AtomicBool>>,
        mut on_progress: F,
    ) -> Result<u64, Error>
    where
        F: FnMut(u64, u64) + Send,
    {
        if let Some(dir) = dest.parent() {
            if !dir.as_os_str().is_empty() {
                tokio::fs::create_dir_all(dir).await?;
            }
        }
        let part = part_path(dest);
        let expected = link.size.max(0) as u64;

        // 用一个内部循环处理一次完整传输: 先带 Range 试探续传, 若服务端不认
        // Range(200) 或 part 已失效(416) 则截断从头再来。真正的网络中断错误会
        // 抛给调用方去退避重试(届时凭残留 part 自动续传)。
        let mut downloaded = 0u64;
        for _pass in 0..2 {
            let existing = tokio::fs::metadata(&part)
                .await
                .map(|m| m.len())
                .unwrap_or(0);

            let (token, _, user_id, _) = self.current_auth().await;
            // 直链(web_content_link)是签名链接, 一般无需带鉴权头即可下载; 若服务端
            // 要求登录(401/403)则补上 Bearer 再试一次(api 域名的流媒体链接会用到)。
            let build = |existing: u64, auth: bool| {
                let mut req = self
                    .http
                    .get(&link.url)
                    .header("User-Agent", build_user_agent(&self.device_id, &user_id))
                    .header("X-Device-Id", &self.device_id);
                if auth {
                    req = req.bearer_auth(&token);
                }
                if existing > 0 {
                    req = req.header("Range", format!("bytes={existing}-"));
                }
                req
            };
            let mut resp = build(existing, false).send().await?;
            let mut status = resp.status();
            if (status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN)
                && !token.is_empty()
            {
                resp = build(existing, true).send().await?;
                status = resp.status();
            }

            match status {
                reqwest::StatusCode::PARTIAL_CONTENT if existing > 0 => {
                    // 服务端认了 Range: 追加写。
                    let mut f = tokio::fs::OpenOptions::new()
                        .append(true)
                        .open(&part)
                        .await?;
                    let n = stream_body(
                        resp,
                        &mut f,
                        expected,
                        existing,
                        cancel.clone(),
                        &mut on_progress,
                    )
                    .await?;
                    f.flush().await?;
                    drop(f);
                    downloaded = n;
                    break;
                }
                reqwest::StatusCode::RANGE_NOT_SATISFIABLE if existing > 0 => {
                    if expected > 0 && existing >= expected {
                        // part 已经完整: 直接收尾。
                        tokio::fs::rename(&part, dest).await?;
                        return Ok(existing);
                    }
                    // part 比远端还大/远端变短, 已失效: 截断, 下一轮从头下。
                    tokio::fs::File::create(&part).await?;
                    continue;
                }
                s if s.is_success() => {
                    // 200: Range 被忽略或全新下载, 从头写(截断旧 part)。
                    let mut f = tokio::fs::File::create(&part).await?;
                    let n =
                        stream_body(resp, &mut f, expected, 0, cancel.clone(), &mut on_progress)
                            .await?;
                    f.flush().await?;
                    drop(f);
                    downloaded = n;
                    break;
                }
                _ => {
                    let text = resp.text().await.unwrap_or_default();
                    return Err(Error::HttpStatus {
                        status: status.as_u16(),
                        body: truncate(&text, 200),
                    });
                }
            }
        }

        (on_progress)(expected, downloaded);
        // 完整性校验: 若服务端声明了大小, 收到的不应比声明少(否则视为被截断,
        // 保留 part 交由调用方续传; 不会把残缺文件改名成正式文件)。
        if expected > 0 && downloaded < expected {
            return Err(Error::transient(format!(
                "传输不完整: 期望 {expected} 字节, 仅收到 {downloaded} 字节(可重试续传)"
            )));
        }
        tokio::fs::rename(&part, dest).await?;
        Ok(downloaded)
    }

    // ---------- 本地上传 ----------

    /// 创建上传票据(`POST /drive/v1/files`)。
    ///
    /// `hash` 为 gcid(会转大写); 返回 `UploadTicket`, 若 `completed` 则秒传命中,
    /// 否则用返回的 OSS 上下文做分片上传。captcha 的 code9 恢复由 `request_inner` 处理。
    pub async fn upload_create(
        &self,
        name: &str,
        parent_id: Option<&str>,
        size: u64,
        hash: &str,
    ) -> Result<UploadTicket, Error> {
        let url = format!("{API_HOST}/drive/v1/files");
        let mut body = serde_json::Map::new();
        body.insert("kind".into(), "drive#file".into());
        body.insert("name".into(), name.into());
        body.insert("size".into(), size.to_string().into());
        body.insert("hash".into(), hash.to_ascii_uppercase().into());
        body.insert("upload_type".into(), "UPLOAD_TYPE_RESUMABLE".into());
        body.insert(
            "objProvider".into(),
            json!({ "provider": "UPLOAD_TYPE_UNKNOWN" }),
        );
        if let Some(pid) = parent_id {
            if !pid.is_empty() {
                body.insert("parent_id".into(), pid.into());
            }
        }
        let value = self.post(&url, &Value::Object(body)).await?;
        Ok(UploadTicket::from_response(&value))
    }

    /// 把本地文件按 OSS 分片上传。需要已由 `upload_create` 得到 `oss` 上下文。
    ///
    /// `cancel` 置位时在分片边界中止; `on_progress(已传字节, 总大小)` 每片回调一次。
    /// 返回已上传字节数。分片为顺序上传, 任一步出错由调用方决定是否整体重试。
    pub async fn upload_oss<F>(
        &self,
        oss: &OssContext,
        path: &StdPath,
        cancel: Option<Arc<AtomicBool>>,
        mut on_progress: F,
    ) -> Result<u64, Error>
    where
        F: FnMut(u64, u64) + Send,
    {
        let mut file = tokio::fs::File::open(path).await?;
        let size = file.metadata().await?.len();
        let chunk = upload::upload_chunk_size(size);
        let total_parts = if size == 0 { 1 } else { size.div_ceil(chunk) };

        let upload_id = self.oss_initiate(oss).await?;
        let mut etags: Vec<String> = Vec::with_capacity(total_parts as usize);
        let mut done = 0u64;

        for part in 1..=total_parts {
            if cancel.as_ref().is_some_and(|c| c.load(Ordering::Relaxed)) {
                return Err(Error::msg("上传已取消"));
            }
            let offset = (part - 1) * chunk;
            let len = (size.saturating_sub(offset)).min(chunk) as usize;
            let mut buf = vec![0u8; len];
            if len > 0 {
                file.seek(SeekFrom::Start(offset)).await?;
                file.read_exact(&mut buf).await?;
            }
            let etag = self.oss_upload_part(oss, &upload_id, part, buf).await?;
            etags.push(etag);
            done += len as u64;
            on_progress(done, size);
        }

        self.oss_complete(oss, &upload_id, &etags).await?;
        Ok(done)
    }

    /// 初始化 OSS 分片上传, 返回 `UploadId`。
    async fn oss_initiate(&self, oss: &OssContext) -> Result<String, Error> {
        let query = "uploads";
        let date = upload::http_date_now();
        let auth = upload::oss_authorization("POST", &date, oss, query);
        let url = format!("https://{}/{}?uploads", oss.endpoint.trim_end_matches('/'), oss.key);
        let resp = self
            .http
            .post(&url)
            .header("Date", &date)
            .header("Content-Type", "application/octet-stream")
            .header("x-oss-security-token", &oss.security_token)
            .header("Authorization", auth)
            .header("User-Agent", OSS_UA)
            .send()
            .await?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(Error::HttpStatus {
                status: status.as_u16(),
                body: truncate(&text, 200),
            });
        }
        upload::extract_xml_tag(&text, "UploadId")
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::msg("OSS 初始化响应缺少 UploadId"))
    }

    /// 上传一个分片, 返回其 ETag(不含引号)。
    async fn oss_upload_part(
        &self,
        oss: &OssContext,
        upload_id: &str,
        part_number: u64,
        data: Vec<u8>,
    ) -> Result<String, Error> {
        let query = format!("partNumber={part_number}&uploadId={upload_id}");
        let date = upload::http_date_now();
        let auth = upload::oss_authorization("PUT", &date, oss, &query);
        let url = format!(
            "https://{}/{}?{query}",
            oss.endpoint.trim_end_matches('/'),
            oss.key
        );
        let resp = self
            .http
            .put(&url)
            .header("Date", &date)
            .header("Content-Type", "application/octet-stream")
            .header("x-oss-security-token", &oss.security_token)
            .header("Authorization", auth)
            .header("User-Agent", OSS_UA)
            .body(data)
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::HttpStatus {
                status: status.as_u16(),
                body: truncate(&text, 200),
            });
        }
        resp.headers()
            .get("ETag")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.trim_matches('"').to_string())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::msg("OSS 分片响应缺少 ETag"))
    }

    /// 完成 OSS 分片上传。
    async fn oss_complete(
        &self,
        oss: &OssContext,
        upload_id: &str,
        etags: &[String],
    ) -> Result<(), Error> {
        let query = format!("uploadId={upload_id}");
        let date = upload::http_date_now();
        let auth = upload::oss_authorization("POST", &date, oss, &query);
        let url = format!(
            "https://{}/{}?{query}",
            oss.endpoint.trim_end_matches('/'),
            oss.key
        );
        let body = upload::complete_multipart_body(etags);
        let resp = self
            .http
            .post(&url)
            .header("Date", &date)
            .header("Content-Type", "application/octet-stream")
            .header("x-oss-security-token", &oss.security_token)
            .header("Authorization", auth)
            .header("User-Agent", OSS_UA)
            .body(body)
            .send()
            .await?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(Error::HttpStatus {
                status: status.as_u16(),
                body: truncate(&text, 200),
            });
        }
        // OSS 失败时也可能返回 200 + <Error> 体, 简单探测一下。
        if text.contains("<Error>") && !text.contains("<ETag>") && !text.contains("<CompleteMultipartUploadResult") {
            return Err(Error::msg(format!(
                "OSS 完成分片失败: {}",
                truncate(&text, 200)
            )));
        }
        Ok(())
    }
}

fn truncate(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// batchMove / batchCopy 的请求体。目标目录缺省时 `to` 为空对象(表示根目录)。
fn move_copy_body(ids: &[String], to_parent_id: Option<&str>) -> Value {
    let mut to = serde_json::Map::new();
    if let Some(pid) = to_parent_id {
        if !pid.is_empty() {
            to.insert("parent_id".into(), pid.into());
        }
    }
    json!({ "ids": ids, "to": Value::Object(to) })
}

/// 由请求推导 captcha action, 形如 "GET:/drive/v1/files"。
fn request_action(method: &reqwest::Method, url: &str) -> String {
    let after = url.find("://").map(|i| &url[i + 3..]).unwrap_or(url);
    let path = after.find('/').map(|i| &after[i..]).unwrap_or("");
    format!("{method}:{path}")
}

/// 把响应体流式写入 `out`, 返回最终已写入字节数(含续传起点 `start`)。
///
/// 分片粒度由 reqwest 决定; 每 `FLUSH_INTERVAL` 字节或结束时 flush 一次磁盘。
/// `cancel` 置位时在下一个分片到达处返回 Err(已取消), 已写入内容保留在 part 中。
async fn stream_body<F>(
    resp: reqwest::Response,
    out: &mut tokio::fs::File,
    total: u64,
    start: u64,
    cancel: Option<Arc<AtomicBool>>,
    on_progress: &mut F,
) -> Result<u64, Error>
where
    F: FnMut(u64, u64) + Send,
{
    let mut stream = resp.bytes_stream();
    let mut written = start;
    let mut last_flush = start;
    while let Some(chunk) = stream.next().await {
        if let Some(cancel) = &cancel {
            if cancel.load(Ordering::Relaxed) {
                out.flush().await?;
                return Err(Error::msg("下载已取消"));
            }
        }
        let chunk = chunk?;
        out.write_all(&chunk).await?;
        written += chunk.len() as u64;
        if written.saturating_sub(last_flush) >= FLUSH_INTERVAL {
            out.flush().await?;
            last_flush = written;
        }
        (on_progress)(total, written);
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_list_parse() {
        let json = json!({
            "files": [
                { "id": "f1", "kind": "drive#folder", "name": "A", "size": "0",
                  "created_time": "2024-01-01T00:00:00.000Z", "modified_time": "x",
                  "parent_id": "", "trashed": false },
                { "id": "f2", "kind": "drive#file", "name": "b.mp4", "size": 123456,
                  "parent_id": "f1", "trashed": false }
            ],
            "next_page_token": ""
        });
        let list: FileList = serde_json::from_value(json).unwrap();
        assert_eq!(list.files.len(), 2);
        assert!(list.next_page_token.is_none());
        assert!(list.files[0].is_folder());
        assert_eq!(list.files[0].size, 0);
        assert_eq!(list.files[1].size, 123456);
    }

    #[test]
    fn tasks_parse() {
        let json = json!({
            "tasks": [
                { "id": "t1", "name": "x.iso", "file_id": "f9", "file_size": "1024" }
            ],
            "next_page_token": "next"
        });
        let tasks = Tasks::parse(json);
        assert_eq!(tasks.tasks.len(), 1);
        assert_eq!(tasks.next_page_token.as_deref(), Some("next"));
        assert_eq!(task_name(&tasks.tasks[0]).as_deref(), Some("x.iso"));
        assert_eq!(task_size(&tasks.tasks[0]), Some(1024));
        assert_eq!(task_id(&tasks.tasks[0]).as_deref(), Some("t1"));
    }

    #[tokio::test]
    async fn login_requires_no_refresh_token() {
        // 未登录时若触发 code16 不应调用 refresh(直接报错)。
        let client = PikPakClient::new("0123456789abcdef0123456789abcdef".into());
        let e = client.refresh_token().await.unwrap_err();
        assert!(e.to_string().contains("缺少 refresh token"));
    }

    #[test]
    fn move_copy_body_builds_expected_json() {
        let ids = vec!["a".to_string(), "b".to_string()];
        let v = move_copy_body(&ids, Some("target"));
        assert_eq!(v["ids"], json!(["a", "b"]));
        assert_eq!(v["to"]["parent_id"], "target");
        // 缺省目标 -> to 为空对象(表示根目录)。
        let root = move_copy_body(&ids, None);
        assert!(root["to"].as_object().unwrap().is_empty());
    }
}
