//! 会话：唯一持有令牌、负责登录 / 自动刷新 / 持久化的地方。
#![allow(dead_code)]
//!
//! 纪律（为将来拆 daemon 留路）：QML 只消费本模块产出的数据；
//! 所有阻塞网络调用都经 `run_async` 丢到后台线程，结果回到 Qt 线程。

pub mod download;
mod download_naming;
mod download_resume;
pub mod media;
pub mod partials;
pub mod paths;
pub mod search;
mod shards;
mod store;
pub mod subtitle_cache;
pub mod subtitles;
mod tasks;

mod client_cache;
mod refresh;

pub use paths::{config_dir, store_path, transfers_path};
pub use search::{ScanBatch, ScanEnd, SearchHit};
pub use tasks::has_active;

mod failure;

pub use failure::{rate_limit_text, Failure};

#[cfg(test)]
mod live_tests;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;

use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};

use client_cache::ClientCache;
use kichi_core::{auth, captcha, Config, DriveClient, FileEntry, Quota, REFRESH_MARGIN};

/// 目录最多翻 40 页（20,000 条），防止异常分页把内存打满。
const MAX_LIST_PAGES: usize = 40;
const LIST_PAGE_SIZE: usize = 500;

pub use store::{Backend, StoreError};

/// 落盘的会话数据（钥匙串或降级文件）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedSession {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_unix: i64,
    pub device_id: String,
    pub user_id: String,
    #[serde(default)]
    pub email: String,
}

impl PersistedSession {
    fn expires_at(&self) -> DateTime<Utc> {
        Utc.timestamp_opt(self.expires_unix, 0)
            .single()
            .unwrap_or_else(Utc::now)
    }

    fn tokens(&self) -> kichi_core::TokenPair {
        kichi_core::TokenPair {
            access_token: self.access_token.clone(),
            refresh_token: self.refresh_token.clone(),
            expires_at: self.expires_at(),
        }
    }
}

/// 令牌状态机。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// 未登录
    SignedOut,
    /// 有可用令牌（或即将自动刷新）
    SignedIn,
    /// 令牌失效 / 被拒，需要重新登录
    Expired,
}

/// 会话门面。
pub struct Session {
    config: Config,
    store_path: PathBuf,
    inner: Mutex<Inner>,
    /// 网盘客户端缓存（按 access token）：连接池与 TLS session 都在里面
    clients: ClientCache,
    /// 令牌刷新单飞（并发下载线程同时发现临期时只刷一次）
    refresh_lock: Mutex<()>,
    /// 测试注入口：换成假刷新实现，验证只发一次请求
    #[cfg(test)]
    refresh_hook: Mutex<Option<Arc<refresh::RefreshHook>>>,
    /// 测试里禁止写真实钥匙串（假令牌绝不能覆盖用户的会话）
    #[cfg(test)]
    store_disabled: bool,
}

struct Inner {
    current: Option<PersistedSession>,
    device_id: String,
}

impl Session {
    /// 启动时调用：加载配置、恢复（并必要时刷新）会话。
    ///
    /// `store_path` 是钥匙串不可用时的降级文件位置。
    /// 返回 `(会话, 恢复状态, 存储后端)`；后端为 `None` 表示未读写过存储。
    pub fn bootstrap(store_path: PathBuf) -> (Self, RestoredState) {
        let config = Config::from_env();
        let (persisted, backend) = match store::load(&store_path) {
            Ok(Some((session, backend))) => (Some(session), Some(backend)),
            Ok(None) => (None, None),
            Err(err) => {
                let state = RestoredState {
                    state: State::SignedOut,
                    backend: None,
                    notice: Some(err.to_string()),
                    email: None,
                };
                let session = Self::new(config, store_path, None, String::new());
                return (session, state);
            }
        };

        let device_id = persisted
            .as_ref()
            .map(|p| p.device_id.clone())
            .filter(|id| id.len() == 32)
            .unwrap_or_else(captcha::generate_device_id);

        let session = Self::new(config, store_path, persisted.clone(), device_id);
        let restored = match persisted {
            None => RestoredState {
                state: State::SignedOut,
                backend,
                notice: None,
                email: None,
            },
            Some(saved) => {
                let email = (!saved.email.is_empty()).then(|| saved.email.clone());
                let tokens = saved.tokens();
                if tokens.needs_refresh(REFRESH_MARGIN) {
                    // 启动即刷新：失败就要求重新登录
                    match auth::refresh(&session.config, &session.device_id(), &saved.refresh_token)
                    {
                        Ok(new_tokens) => {
                            let notice = session
                                .replace_tokens(new_tokens)
                                .err()
                                .map(|e| e.to_string());
                            RestoredState {
                                state: session.state(),
                                backend,
                                notice,
                                email,
                            }
                        }
                        Err(err) => {
                            let _ = store::clear(&session.store_path);
                            session.clear_tokens();
                            RestoredState {
                                state: State::Expired,
                                backend,
                                notice: Some(Failure::from(err).to_string()),
                                email,
                            }
                        }
                    }
                } else {
                    RestoredState {
                        state: State::SignedIn,
                        backend,
                        notice: None,
                        email,
                    }
                }
            }
        };
        (session, restored)
    }

    fn new(
        config: Config,
        store_path: PathBuf,
        current: Option<PersistedSession>,
        device_id: String,
    ) -> Self {
        Self {
            config,
            store_path,
            inner: Mutex::new(Inner { current, device_id }),
            clients: ClientCache::default(),
            refresh_lock: Mutex::new(()),
            #[cfg(test)]
            refresh_hook: Mutex::new(None),
            #[cfg(test)]
            store_disabled: false,
        }
    }

    /// 借出内部状态。
    ///
    /// 锁中毒时继续使用内部数据：`Inner` 只是纯数据结构（令牌 + 设备 id），
    /// 没有「改到一半」的中间态，恢复比 panic 更安全——UI 也拿不到一把坏锁。
    fn with_inner<T>(&self, f: impl FnOnce(&mut Inner) -> T) -> T {
        let mut guard = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        f(&mut guard)
    }

    pub fn state(&self) -> State {
        self.with_inner(|inner| match &inner.current {
            None => State::SignedOut,
            Some(saved) if saved.tokens().is_expired() => State::Expired,
            Some(_) => State::SignedIn,
        })
    }

    pub fn email(&self) -> Option<String> {
        self.with_inner(|inner| {
            inner
                .current
                .as_ref()
                .map(|s| s.email.clone())
                .filter(|e| !e.is_empty())
        })
    }

    pub fn device_id(&self) -> String {
        self.with_inner(|inner| inner.device_id.clone())
    }

    /// 登录（阻塞；调用方应放后台线程）。成功后会自动落盘。
    pub fn sign_in(
        &self,
        username: &str,
        password: &str,
        manual_captcha: Option<&str>,
    ) -> Result<(), Failure> {
        let device_id = self.device_id();
        let outcome = auth::sign_in(&self.config, &device_id, username, password, manual_captcha)?;

        let saved = PersistedSession {
            access_token: outcome.tokens.access_token.clone(),
            refresh_token: outcome.tokens.refresh_token.clone(),
            expires_unix: outcome.tokens.expires_at.timestamp(),
            device_id: device_id.clone(),
            user_id: outcome.user_id.clone(),
            email: username.trim().to_owned(),
        };
        store::save(&self.store_path, &saved)?;
        // 可能是换账号登录：旧账号的 client（以及里面的连接）不能留着
        self.clients.clear();
        self.with_inner(|inner| inner.current = Some(saved));
        Ok(())
    }

    /// 用更完整的身份信息补全会话（`/v1/user/me`，尽力而为）。
    pub fn enrich_identity(&self, name: &str, email: &str) {
        let label = if !email.is_empty() { email } else { name };
        if label.is_empty() {
            return;
        }
        let updated = self.with_inner(|inner| {
            let saved = inner.current.as_mut()?;
            if saved.email == label {
                return None;
            }
            saved.email = label.to_owned();
            Some(saved.clone())
        });
        if let Some(saved) = updated {
            if let Err(err) = store::save(&self.store_path, &saved) {
                eprintln!("[kichi] 会话信息更新未能落盘：{err}");
            }
        }
    }

    fn replace_tokens(&self, tokens: kichi_core::TokenPair) -> Result<(), StoreError> {
        // 令牌一换，按令牌缓存的网盘 client 就作废了（换账号不可能复用）
        self.clients.clear();
        let updated = self.with_inner(|inner| {
            let saved = inner.current.as_mut()?;
            saved.access_token = tokens.access_token;
            saved.refresh_token = tokens.refresh_token;
            saved.expires_unix = tokens.expires_at.timestamp();
            Some(saved.clone())
        });
        let Some(saved) = updated else {
            return Ok(());
        };
        #[cfg(test)]
        if self.store_disabled {
            return Ok(());
        }
        store::save(&self.store_path, &saved)?;
        Ok(())
    }

    fn clear_tokens(&self) {
        // 登出 / 令牌失效：缓存里的 client 连同连接池一起放掉
        self.clients.clear();
        self.with_inner(|inner| inner.current = None);
    }

    /// 登出：清空内存与持久化。
    pub fn sign_out(&self) -> Result<(), StoreError> {
        store::clear(&self.store_path)?;
        self.clear_tokens();
        Ok(())
    }

    /// 当前令牌与设备身份（未登录 → Auth 失败）。
    fn current_credentials(&self) -> Result<(String, String, String), Failure> {
        let current = self.with_inner(|inner| {
            inner.current.as_ref().map(|s| {
                (
                    s.access_token.clone(),
                    s.user_id.clone(),
                    inner.device_id.clone(),
                )
            })
        });
        current.ok_or(Failure::Auth("尚未登录".into()))
    }

    /// 构造网盘客户端（阻塞调用方：可能在后台线程）。令牌失败自动刷新一次。
    ///
    /// 同一个 access token 复用同一个 client（连接池 + TLS 不重建）；
    /// 令牌一换就重建，因此长批次跨刷新也安全。
    pub fn drive(&self) -> Result<Arc<DriveClient>, Failure> {
        if let Err(err) = self.ensure_tokens() {
            if matches!(err, Failure::Auth(_)) {
                let _ = store::clear(&self.store_path);
                self.clear_tokens();
            }
            return Err(err);
        }
        let (access, user_id, device_id) = self.current_credentials()?;
        if let Some(client) = self.clients.get(&access) {
            return Ok(client);
        }
        let built = Arc::new(
            DriveClient::new(&self.config, &device_id, &access, &user_id).map_err(Failure::from)?,
        );
        Ok(self.clients.install(access, built))
    }

    /// 取当前容量（阻塞）。
    pub fn quota(&self) -> Result<Quota, Failure> {
        let client = self.drive()?;
        client.about().map_err(Failure::from)
    }

    /// 读取一个目录的全部条目（自动翻页；阻塞）。
    ///
    /// `parent_id` 为 `None` 表示根目录。条目顺序保留官方返回顺序，
    /// 排序交给界面层（文件夹优先等属于展示策略）。
    pub fn list_dir(&self, parent_id: Option<&str>) -> Result<Vec<FileEntry>, Failure> {
        self.list_dir_with_cancel(parent_id, None)
    }

    /// 同 [`Session::list_dir`]，但每页之间查一次取消信号。
    ///
    /// 文件夹下载要先递归读完整棵目录树，大文件夹可能要好几十秒；
    /// 这期间按「取消下载」必须能尽快停下来（见 `cxxqt_object/queue.rs`）。
    pub fn list_dir_with_cancel(
        &self,
        parent_id: Option<&str>,
        cancel: Option<&download::Cancel>,
    ) -> Result<Vec<FileEntry>, Failure> {
        let client = self.drive()?;
        let mut entries = Vec::new();
        let mut page_token: Option<String> = None;
        for _ in 0..MAX_LIST_PAGES {
            if cancel.is_some_and(download::Cancel::is_cancelled) {
                return Err(Failure::Other("下载已取消".to_owned()));
            }
            let (mut page, next) = client
                .list(parent_id, page_token.as_deref())
                .map_err(Failure::from)?;
            entries.append(&mut page);
            match next {
                Some(token) => page_token = Some(token),
                None => return Ok(entries),
            }
        }
        Err(Failure::Other(format!(
            "目录条目超过 {} 条，暂不支持完整显示",
            MAX_LIST_PAGES * LIST_PAGE_SIZE
        )))
    }

    /// 取当前用户并回填身份（阻塞）。
    pub fn refresh_identity(&self) -> Result<(), Failure> {
        let client = self.drive()?;
        let me = client.me().map_err(Failure::from)?;
        self.enrich_identity(&me.name, &me.email);
        Ok(())
    }

    /// 给条目改名（阻塞）。
    pub fn rename_entry(&self, id: &str, name: &str) -> Result<(), Failure> {
        let client = self.drive()?;
        client.rename(id, name).map_err(Failure::from)
    }

    /// 在当前目录下新建文件夹（阻塞）。`parent_id` 为空表示根目录。
    pub fn create_folder(&self, parent_id: Option<&str>, name: &str) -> Result<(), Failure> {
        let client = self.drive()?;
        client.create_folder(parent_id, name).map_err(Failure::from)
    }

    /// 把条目移入回收站（阻塞）。只做回收站，不做彻底删除。
    pub fn trash_entries(&self, ids: &[&str]) -> Result<(), Failure> {
        let client = self.drive()?;
        client.trash(ids).map_err(Failure::from)
    }
}

/// 启动恢复的结果。
#[derive(Debug)]
pub struct RestoredState {
    pub state: State,
    pub backend: Option<Backend>,
    pub notice: Option<String>,
    pub email: Option<String>,
}
