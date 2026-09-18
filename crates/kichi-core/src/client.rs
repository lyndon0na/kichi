use std::collections::{HashSet, VecDeque};
use std::path::Path as StdPath;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use futures_util::stream::{self, StreamExt};
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;
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
use crate::upload::{self, OssContext, OssUploadState, UploadTicket};

/// 写入缓冲的落盘阈值: 每攒够这么多字节 flush 一次磁盘。
const FLUSH_INTERVAL: u64 = 8 * 1024 * 1024;

const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
    (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36";

/// OSS 请求使用的 User-Agent(沿用可用的阿里云 SDK 风格)。
const OSS_UA: &str = "aliyun-sdk-android/2.9.5";

/// OSS 分片并发的分片数。
const OSS_UPLOAD_CONCURRENCY: usize = 4;

/// 整目录下载递归遍历时的页大小与文件总数上限(超限返回错误, 避免异常数据失控)。
const WALK_PAGE_SIZE: usize = 100;
const WALK_MAX_FILES: usize = 20_000;

#[derive(Default)]
struct Auth {
    access_token: String,
    refresh_token: String,
    user_id: String,
    username: String,
    captcha_token: Option<String>,
}

pub type TokenSaver = Arc<dyn Fn(&Session) + Send + Sync>;

pub struct KichiClient {
    http: reqwest::Client,
    device_id: String,
    auth: Mutex<Auth>,
    /// 续期单飞: 保证同一时刻只有一个请求在刷新 token, 避免并发刷新与重复写盘。
    refresh_lock: Mutex<()>,
    on_tokens: Option<TokenSaver>,
}

impl KichiClient {
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
            return Err(Error::AuthExpired(
                if err_body.error_description.is_empty() {
                    err_body.error
                } else {
                    err_body.error_description
                },
            ));
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
        let filters = default_file_filters();
        self.file_list_filtered(filters, parent_id, size, next_page_token)
            .await
    }

    /// 递归列出 `folder_id` 下的全部子目录与文件(整目录下载用)。
    ///
    /// 目录路径以「相对根目录的组件序列」表示(根为空 vec), 便于调用方在本地按
    /// 云端层级建目录; 空目录也会出现在 `dirs` 中。用已访问 id 集合防环。
    pub async fn walk_folder(&self, folder_id: &str) -> Result<FolderWalk, Error> {
        let root = folder_id.trim();
        if root.is_empty() {
            return Err(Error::msg("目录 ID 为空"));
        }
        let mut walk = FolderWalk {
            dirs: vec![Vec::new()],
            files: Vec::new(),
        };
        let mut seen: HashSet<String> = HashSet::new();
        seen.insert(root.to_string());
        // (目录 id, 相对组件)
        let mut queue: VecDeque<(String, Vec<String>)> = VecDeque::new();
        queue.push_back((root.to_string(), Vec::new()));

        while let Some((id, rel)) = queue.pop_front() {
            let mut token: Option<String> = None;
            loop {
                let FileList {
                    files,
                    next_page_token,
                } = self
                    .file_list(Some(&id), WALK_PAGE_SIZE, token.as_deref())
                    .await?;
                for f in files {
                    if f.is_folder() {
                        if seen.insert(f.id.clone()) {
                            let mut child_rel = rel.clone();
                            child_rel.push(f.name.clone());
                            walk.dirs.push(child_rel.clone());
                            queue.push_back((f.id.clone(), child_rel));
                        }
                    } else {
                        if walk.files.len() >= WALK_MAX_FILES {
                            return Err(Error::msg(format!(
                                "目录内文件过多(超过 {WALK_MAX_FILES} 个), 暂不支持整目录下载"
                            )));
                        }
                        walk.files.push((rel.clone(), f));
                    }
                }
                match next_page_token {
                    Some(t) => token = Some(t),
                    None => break,
                }
            }
        }
        Ok(walk)
    }

    /// 列出回收站内容(filters `trashed.eq=true`)。
    ///
    /// 注意: 回收站是跨目录的全局列表, 必须显式传 `parent_id=*`(官方客户端行为),
    /// 否则服务端按当前目录过滤, 会返回空列表。
    pub async fn trash_list(
        &self,
        size: usize,
        next_page_token: Option<&str>,
    ) -> Result<FileList, Error> {
        let filters = trashed_file_filters();
        self.file_list_filtered(filters, Some("*"), size, next_page_token)
            .await
    }

    /// 按给定 filters 列出文件(分页)。
    async fn file_list_filtered(
        &self,
        filters: Value,
        parent_id: Option<&str>,
        size: usize,
        next_page_token: Option<&str>,
    ) -> Result<FileList, Error> {
        let url = format!("{API_HOST}/drive/v1/files");
        let mut query: Vec<(&str, String)> = vec![
            ("thumbnail_size", "SIZE_MEDIUM".into()),
            ("limit", size.to_string()),
            ("with_audit", "true".into()),
            ("filters", filters.to_string()),
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

    /// 新建文件夹并返回其 id。
    pub async fn create_folder_id(
        &self,
        name: &str,
        parent_id: Option<&str>,
    ) -> Result<String, Error> {
        let v = self.create_folder(name, parent_id).await?;
        v.get("file")
            .and_then(|f| f.get("id"))
            .and_then(|i| i.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .ok_or_else(|| Error::msg("创建文件夹响应缺少 id"))
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

    /// 批量从回收站还原。
    pub async fn batch_untrash(&self, ids: &[String]) -> Result<Value, Error> {
        let url = format!("{API_HOST}/drive/v1/files:batchUntrash");
        self.post(&url, &json!({ "ids": ids })).await
    }

    /// 批量彻底删除(不可恢复, 仅对回收站内文件有效)。
    pub async fn batch_delete(&self, ids: &[String]) -> Result<Value, Error> {
        let url = format!("{API_HOST}/drive/v1/files:batchDelete");
        self.post(&url, &json!({ "ids": ids })).await
    }

    /// 服务端「清空回收站」: 一次清空全部内容(无请求体)。
    pub async fn empty_trash(&self) -> Result<(), Error> {
        let url = format!("{API_HOST}/drive/v1/files/trash:empty");
        self.request(reqwest::Method::PATCH, &url, None, &[])
            .await?;
        Ok(())
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

    // ---------- 分享 ----------

    /// 创建分享链接。`expiration_days` 为 -1 表示永久;
    /// `need_password` 为 true 时生成带提取码的私密分享。
    pub async fn share_create(
        &self,
        file_ids: &[String],
        expiration_days: i64,
        need_password: bool,
    ) -> Result<ShareCreated, Error> {
        let url = format!("{API_HOST}/drive/v1/share");
        let body = json!({
            "file_ids": file_ids,
            "share_to": if need_password { "encryptedlink" } else { "publiclink" },
            "expiration_days": expiration_days,
            "pass_code_option": if need_password { "REQUIRED" } else { "NOT_REQUIRED" },
        });
        let value = self.post(&url, &body).await?;
        let created: ShareCreated = serde_json::from_value(value)?;
        if created.share_url.is_empty() {
            return Err(Error::msg("创建分享失败: 响应缺少分享链接"));
        }
        Ok(created)
    }

    /// 列出「我的分享」(单页)。分页游标为空表示没有更多。
    pub async fn share_list(
        &self,
        size: usize,
        next_page_token: Option<&str>,
    ) -> Result<ShareList, Error> {
        let url = format!("{API_HOST}/drive/v1/share/list");
        let mut query: Vec<(&str, String)> = vec![
            ("limit", size.to_string()),
            ("thumbnail_size", "SIZE_SMALL".into()),
        ];
        if let Some(tok) = next_page_token {
            query.push(("page_token", tok.to_string()));
        }
        let value = self.get(&url, &query).await?;
        Ok(serde_json::from_value(value)?)
    }

    /// 批量取消分享(服务端接口为 share:batchDelete)。
    pub async fn share_batch_delete(&self, ids: &[String]) -> Result<(), Error> {
        let url = format!("{API_HOST}/drive/v1/share:batchDelete");
        self.post(&url, &json!({ "ids": ids })).await?;
        Ok(())
    }

    /// 解析他人分享链接, 获取文件列表与 `pass_code_token`(转存时需要)。
    /// `pass_code` 为提取码, 公开分享传空字符串。
    pub async fn share_info(&self, share_id: &str, pass_code: &str) -> Result<ShareDetail, Error> {
        let url = format!("{API_HOST}/drive/v1/share");
        let mut query: Vec<(&str, String)> = vec![
            ("limit", "100".into()),
            ("thumbnail_size", "SIZE_LARGE".into()),
            ("share_id", share_id.to_string()),
        ];
        if !pass_code.is_empty() {
            query.push(("pass_code", pass_code.to_string()));
        }
        let value = self.get(&url, &query).await?;
        let detail: ShareDetail = serde_json::from_value(value)?;
        if !detail.share_status.is_empty() && detail.share_status != "OK" {
            return Err(Error::msg(format!("分享不可用: {}", detail.share_status)));
        }
        Ok(detail)
    }

    /// 分页获取分享内的文件列表(当文件数超过首页 limit 时)。
    pub async fn share_detail(
        &self,
        share_id: &str,
        pass_code_token: &str,
        page_token: &str,
    ) -> Result<ShareDetail, Error> {
        let url = format!("{API_HOST}/drive/v1/share/detail");
        let query: Vec<(&str, String)> = vec![
            ("limit", "100".into()),
            ("thumbnail_size", "SIZE_LARGE".into()),
            ("share_id", share_id.to_string()),
            ("pass_code_token", pass_code_token.to_string()),
            ("page_token", page_token.to_string()),
        ];
        let value = self.get(&url, &query).await?;
        Ok(serde_json::from_value(value)?)
    }

    /// 将分享中的文件转存到自己的网盘。`file_ids` 为要保存的文件/文件夹 id 列表。
    pub async fn share_restore(
        &self,
        share_id: &str,
        pass_code_token: &str,
        file_ids: &[String],
    ) -> Result<ShareRestoreResult, Error> {
        let url = format!("{API_HOST}/drive/v1/share/restore");
        let body = json!({
            "kind": "drive#file",
            "share_id": share_id,
            "pass_code_token": pass_code_token,
            "file_ids": file_ids,
        });
        let value = self.post(&url, &body).await?;
        let result: ShareRestoreResult = serde_json::from_value(value)?;
        if result.restore_status != "RESTORE_START" {
            return Err(Error::msg(format!("转存失败: {}", result.restore_status)));
        }
        Ok(result)
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
        let accept =
            |s: reqwest::StatusCode| s.is_success() || s == reqwest::StatusCode::PARTIAL_CONTENT;

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
        // folder_type 必须为 NORMAL, 否则根目录上传会被服务端放进默认的「My Upload」。
        body.insert("folder_type".into(), "NORMAL".into());
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

    /// 把本地文件按 OSS 分片上传(并发 + 续传)。
    ///
    /// 需要已由 `upload_create` 得到 `oss` 上下文, 并由 `oss_initiate` 得到 `upload_id`。
    /// `state` 保存已成功分片的 ETag; 重试时传入同一 `state` 即可跳过已上传分片。
    /// `cancel` 置位时分片边界中止; `on_progress(已传字节, 总大小)` 随分片完成回调。
    pub async fn upload_oss<F>(
        &self,
        oss: &OssContext,
        upload_id: &str,
        path: &StdPath,
        cancel: Option<Arc<AtomicBool>>,
        state: &mut OssUploadState,
        on_progress: &mut F,
    ) -> Result<u64, Error>
    where
        F: FnMut(u64, u64) + Send,
    {
        let size = tokio::fs::metadata(path).await?.len();
        let chunk = upload::upload_chunk_size(size);
        let total_parts = if size == 0 { 1 } else { size.div_ceil(chunk) };
        let missing: Vec<u64> = (1..=total_parts)
            .filter(|p| !state.etags.contains_key(p))
            .collect();

        let done = Arc::new(AtomicU64::new(state.uploaded_bytes(chunk, size)));
        let cb = Arc::new(std::sync::Mutex::new(on_progress));
        let path = path.to_path_buf();
        let cancel2 = cancel.clone();

        let results = stream::iter(missing.into_iter().map(|part| {
            let path = path.clone();
            let cancel = cancel2.clone();
            let cb = cb.clone();
            let done = done.clone();
            async move {
                if cancel.as_ref().is_some_and(|c| c.load(Ordering::Relaxed)) {
                    return Err(Error::msg("上传已取消"));
                }
                let offset = (part - 1) * chunk;
                let len = upload::part_len(part, chunk, size) as usize;
                let buf = tokio::task::spawn_blocking(move || read_file_range(&path, offset, len))
                    .await
                    .map_err(|e| Error::msg(format!("读取待上传分片失败: {e}")))??;
                let etag = self.oss_upload_part(oss, upload_id, part, buf).await?;
                let now = done.fetch_add(len as u64, Ordering::Relaxed) + len as u64;
                if let Ok(mut g) = cb.lock() {
                    (*g)(now, size);
                }
                Ok::<(u64, String), Error>((part, etag))
            }
        }))
        .buffered(OSS_UPLOAD_CONCURRENCY);

        let mut stream = std::pin::pin!(results);
        while let Some(res) = stream.next().await {
            let (part, etag) = res?;
            state.etags.insert(part, etag);
        }

        if let Ok(mut g) = cb.lock() {
            (*g)(size, size);
        }

        let mut ordered: Vec<(u64, String)> =
            state.etags.iter().map(|(p, e)| (*p, e.clone())).collect();
        ordered.sort_by_key(|(p, _)| *p);
        let etags: Vec<String> = ordered.into_iter().map(|(_, e)| e).collect();
        self.oss_complete(oss, upload_id, &etags).await?;
        Ok(size)
    }

    /// 初始化 OSS 分片上传, 返回 `UploadId`。
    pub async fn oss_initiate(&self, oss: &OssContext) -> Result<String, Error> {
        let query = "uploads";
        let date = upload::http_date_now();
        let auth = upload::oss_authorization("POST", &date, oss, query);
        let url = format!(
            "https://{}/{}?uploads",
            oss.endpoint.trim_end_matches('/'),
            oss.key
        );
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
        if text.contains("<Error>")
            && !text.contains("<ETag>")
            && !text.contains("<CompleteMultipartUploadResult")
        {
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

/// 读取文件的一个字节区间(供分片上传在阻塞线程中调用)。
fn read_file_range(path: &StdPath, offset: u64, len: usize) -> std::io::Result<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path)?;
    let mut buf = vec![0u8; len];
    if len > 0 {
        f.seek(SeekFrom::Start(offset))?;
        f.read_exact(&mut buf)?;
    }
    Ok(buf)
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
        let client = KichiClient::new("0123456789abcdef0123456789abcdef".into());
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

    #[test]
    fn trashed_filters_mark_trash() {
        let f = trashed_file_filters();
        assert_eq!(f["trashed"]["eq"], json!(true));
        // 回收站列表不应混入 phase 过滤(被删的目录/未完成项也要能列出)。
        assert!(f.get("phase").is_none());
    }

    #[test]
    fn share_list_parse() {
        // 计数类字段既有字符串也有数字, 都应能解析。
        let json = json!({
            "data": [
                {
                    "share_id": "S1",
                    "share_url": "https://mypikpak.com/s/S1",
                    "title": "我的分享",
                    "pass_code": "abcd",
                    "share_to": "encryptedlink",
                    "file_num": "3",
                    "view_count": 10,
                    "restore_count": "1",
                    "expiration_days": "-1",
                    "share_status": "OK"
                }
            ],
            "next_page_token": "next"
        });
        let list: ShareList = serde_json::from_value(json).unwrap();
        assert_eq!(list.shares.len(), 1);
        assert_eq!(list.next_page_token.as_deref(), Some("next"));
        let s = &list.shares[0];
        assert_eq!(s.share_id, "S1");
        assert_eq!(s.view_count, "10");
        assert!(s.needs_pass_code());
        assert_eq!(s.expiry_label(), "永久");
        assert_eq!(s.file_count(), 3);
        assert!(!s.is_unavailable());
    }

    #[test]
    fn share_list_defaults_and_empty_token() {
        let json = json!({ "data": [], "next_page_token": "" });
        let list: ShareList = serde_json::from_value(json).unwrap();
        assert!(list.shares.is_empty());
        assert!(list.next_page_token.is_none());
    }

    #[test]
    fn share_created_parse() {
        let json = json!({
            "share_id": "S2",
            "share_url": "https://mypikpak.com/s/S2",
            "pass_code": "",
            "share_text": "分享给你"
        });
        let c: ShareCreated = serde_json::from_value(json).unwrap();
        assert_eq!(c.share_id, "S2");
        assert_eq!(c.pass_code, "");
        assert_eq!(c.share_text, "分享给你");
    }
}
