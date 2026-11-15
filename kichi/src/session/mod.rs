//! 会话：唯一持有令牌、负责登录 / 自动刷新 / 持久化的地方。
#![allow(dead_code)]
//!
//! 纪律（为将来拆 daemon 留路）：QML 只消费本模块产出的数据；
//! 所有阻塞网络调用都经 `run_async` 丢到后台线程，结果回到 Qt 线程。

pub mod avatar_cache;
pub mod download;
mod download_naming;
mod download_resume;
pub(crate) mod file_cache;
mod folder_trail;
mod identity;
pub mod image_preview;
pub(crate) mod library;
pub(crate) mod limits;
pub mod magnet;
pub mod media;
pub(crate) mod page_snapshot;
pub mod partials;
pub mod paths;
mod playback_history;
pub(crate) mod private_fs;
pub mod search;
mod shards;
mod share_open;
mod shares;
mod store;
pub mod subtitle_cache;
pub mod subtitles;
mod tasks;
mod throttle;
pub mod thumbnail_cache;
pub(crate) mod tree_index;
pub mod upload;
pub(crate) mod upload_resume;

mod client_cache;
mod refresh;

pub use paths::{config_dir, store_path, transfers_path};
pub use search::{IndexSearchResult, ScanEnd, ScanProgress, SearchHit};
pub use tasks::has_active;

mod failure;

pub use failure::{rate_limit_text, Failure};

#[cfg(test)]
mod live_tests;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};

use client_cache::ClientCache;
use kichi_core::{
    auth, captcha, Config, DriveClient, FileEntry, Quota, TransferQuota, TrashEntry, REFRESH_MARGIN,
};

/// 目录最多翻 40 页（20,000 条），防止异常分页把内存打满。
const MAX_LIST_PAGES: usize = 40;
const LIST_PAGE_SIZE: usize = 500;
/// 静默重登失败后的冷却：官方有风控，不能让每次请求都撞一次登录接口。
const RELOGIN_COOLDOWN: Duration = Duration::from_secs(60);

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
    /// `/v1/user/me` 返回的昵称；旧会话没有这个字段。
    #[serde(default)]
    pub display_name: String,
    /// `/v1/user/me` 返回的头像地址；只供 Rust 下载器使用，不直接交给 QML。
    #[serde(default)]
    pub avatar_url: String,
    /// 上次成功读取的已用容量；启动时先显示，避免账号区闪骨架。
    #[serde(default)]
    pub quota_usage: Option<u64>,
    /// 容量上限；0 表示不限量。仅在 `quota_usage` 有值时有意义。
    #[serde(default)]
    pub quota_limit: u64,
    /// 上次读取到的会员状态，和容量一起先显示。
    #[serde(default)]
    pub quota_vip: bool,
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

/// 服务端拒绝令牌后的恢复结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Recovery {
    /// 只换到了新令牌，界面无需变化
    Refreshed,
    /// 用记住的密码重新登录了
    Relogged,
    /// 没有可用的恢复手段（没记住密码 / 冷却中）：只能回登录页
    Unavailable,
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
    /// 静默重登串行 + 冷却（锁里记录上次尝试时间）
    relogin_lock: Mutex<Option<Instant>>,
    /// 当前账号的本地目录树索引；未登录 / 已登出 / 开关关闭时为 `None`
    tree_index: Mutex<Option<Arc<tree_index::TreeIndex>>>,
    /// 本地索引总开关：关掉后连库文件都不打开（页面快照同库，一起停用）
    index_enabled: AtomicBool,
    /// 最近一次索引打开 / 后台回填失败；界面提示一次后取走
    tree_index_error: Mutex<Option<String>>,
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
    /// `local_index_enabled` 来自设置；关闭时启动期不创建 / 不打开索引库。
    /// 返回 `(会话, 恢复状态)`；存储后端信息在 `RestoredState` 里。
    pub fn bootstrap(store_path: PathBuf, local_index_enabled: bool) -> (Self, RestoredState) {
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
                let session = Self::new_with_index(
                    config,
                    store_path,
                    None,
                    String::new(),
                    local_index_enabled,
                );
                return (session, state);
            }
        };

        let device_id = persisted
            .as_ref()
            .map(|p| p.device_id.clone())
            .filter(|id| id.len() == 32)
            .unwrap_or_else(captcha::generate_device_id);

        let session = Self::new_with_index(
            config,
            store_path,
            persisted.clone(),
            device_id,
            local_index_enabled,
        );
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
                            let failure = Failure::from(err);
                            // 只有「refresh token 被官方拒绝」才算会话真没了；
                            // 断网 / 超时不能把磁盘上的会话也抹掉，否则下次
                            // 启动连重试的机会都没有。
                            if matches!(failure, Failure::Auth(_)) {
                                let _ = store::clear(&session.store_path);
                            }
                            // 内存里保留这份（已过期的）会话，登录态才能显示
                            // Expired 而不是 SignedOut；记住的密码仍可静默恢复。
                            RestoredState {
                                state: State::Expired,
                                backend,
                                notice: Some(failure.to_string()),
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
        if restored.state == State::SignedIn {
            // 启动恢复登录：索引没了 / 版本不认识会在这里重建出空库，
            // 是否需要全量建库由调用方查 `needs_full_build` 决定。
            // 开关关闭时这里直接跳过，不创建也不打开库文件。
            let _ = session.open_tree_index();
        }
        (session, restored)
    }

    fn new(
        config: Config,
        store_path: PathBuf,
        current: Option<PersistedSession>,
        device_id: String,
    ) -> Self {
        Self::new_with_index(config, store_path, current, device_id, true)
    }

    fn new_with_index(
        config: Config,
        store_path: PathBuf,
        current: Option<PersistedSession>,
        device_id: String,
        local_index_enabled: bool,
    ) -> Self {
        Self {
            config,
            store_path,
            inner: Mutex::new(Inner { current, device_id }),
            clients: ClientCache::default(),
            refresh_lock: Mutex::new(()),
            relogin_lock: Mutex::new(None),
            tree_index: Mutex::new(None),
            index_enabled: AtomicBool::new(local_index_enabled),
            tree_index_error: Mutex::new(None),
            #[cfg(test)]
            refresh_hook: Mutex::new(None),
            #[cfg(test)]
            store_disabled: false,
        }
    }

    /// 测试用会话：带一个长期有效的假令牌，不读写钥匙串。
    #[cfg(test)]
    pub(crate) fn for_test(config: Config, access_token: &str) -> Self {
        let device_id = "d".repeat(32);
        let saved = PersistedSession {
            access_token: access_token.to_owned(),
            refresh_token: String::new(),
            expires_unix: Utc::now().timestamp() + 3600,
            device_id: device_id.clone(),
            user_id: "test-user".to_owned(),
            email: String::new(),
            display_name: String::new(),
            avatar_url: String::new(),
            quota_usage: None,
            quota_limit: 0,
            quota_vip: false,
        };
        let mut session = Self::new(config, PathBuf::new(), Some(saved), device_id);
        session.store_disabled = true;
        session
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

    pub fn device_id(&self) -> String {
        self.with_inner(|inner| inner.device_id.clone())
    }

    /// 登录（阻塞；调用方应放后台线程）。成功后会自动落盘。
    pub fn sign_in(
        &self,
        username: &str,
        password: &str,
        manual_captcha: Option<&str>,
        remember_password: bool,
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
            display_name: String::new(),
            avatar_url: String::new(),
            quota_usage: None,
            quota_limit: 0,
            quota_vip: false,
        };
        store::save(&self.store_path, &saved)?;
        self.set_remembered_password(username, password, remember_password);
        // 可能是换账号登录：旧账号的 client（以及里面的连接）不能留着
        self.clients.clear();
        self.with_inner(|inner| inner.current = Some(saved));
        // 索引是缓存：打不开不影响登录，错误留给界面提示一次
        let _ = self.open_tree_index();
        Ok(())
    }

    /// 按勾选状态保存 / 删除记住的密码。
    ///
    /// 密码只进钥匙串；失败不影响已经成功的登录，只记日志（下次登录会再试）。
    fn set_remembered_password(&self, username: &str, password: &str, remember: bool) {
        #[cfg(test)]
        if self.store_disabled {
            return;
        }
        let outcome = if remember {
            store::save_password(username, password)
        } else {
            store::clear_password()
        };
        if let Err(err) = outcome {
            eprintln!("[kichi] 记住密码未能保存：{err}");
        }
    }

    /// 已记住的登录凭据（用户名 + 密码）。钥匙串不可用时返回 `None`。
    pub(crate) fn remembered_login(&self) -> Option<(String, String)> {
        match store::load_password() {
            Ok(login) => login,
            Err(err) => {
                eprintln!("[kichi] 读取记住的密码失败：{err}");
                None
            }
        }
    }

    /// 删除记住的密码（用户取消勾选）。
    pub(crate) fn forget_password(&self) {
        if let Err(err) = store::clear_password() {
            eprintln!("[kichi] 删除记住的密码失败：{err}");
        }
    }

    /// 令牌已被官方拒绝：清掉内存与持久化会话，但保留记住的密码。
    pub(crate) fn expire(&self) {
        let _ = store::clear(&self.store_path);
        self.clear_tokens();
    }

    /// 用记住的密码静默重登一次（仅在令牌临期 / 缺失时动手）。
    ///
    /// 返回 `Ok(true)` = 已恢复登录（含「别的线程刚恢复好」），
    /// `Ok(false)` = 没有记住密码 / 冷却中。冷却期内不重复撞登录接口。
    pub(crate) fn silent_relogin(&self) -> Result<bool, Failure> {
        self.relogin(false)
    }

    /// 用记住的密码强制重登一次。
    ///
    /// 用于「服务端已经明确拒绝令牌，但本地时钟还没到期」：这时不能再用
    /// 临期判断跳过，否则会以为会话没事、实际每个请求都 401。
    pub(crate) fn force_relogin(&self) -> Result<bool, Failure> {
        self.relogin(true)
    }

    fn relogin(&self, force: bool) -> Result<bool, Failure> {
        // 主动登出 / 已经被踢下线后不再自动登录，避免后台请求把用户又登回去
        if !self.has_session() {
            return Ok(false);
        }
        // 没记住密码就不必拿锁（钥匙串读取最贵，放在最前面）
        let Some((username, password)) = self.remembered_login() else {
            return Ok(false);
        };
        // 与并发刷新互斥：refresh_token 会轮换，两边同时提交会让先到的失效
        let _refresh = self
            .refresh_lock
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        self.relogin_locked(&username, &password, force)
    }

    /// 已持有 `refresh_lock` 的重登主体。
    fn relogin_locked(&self, username: &str, password: &str, force: bool) -> Result<bool, Failure> {
        let mut last_attempt = self
            .relogin_lock
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        // 等锁期间别的线程可能已经重登成功
        if !force && !self.tokens_need_refresh() {
            return Ok(true);
        }
        if last_attempt.is_some_and(|at| at.elapsed() < RELOGIN_COOLDOWN) {
            return Ok(false);
        }
        *last_attempt = Some(Instant::now());
        self.sign_in(username, password, None, true)?;
        Ok(true)
    }

    /// 当前 access token；未登录时为 `None`（给「是否已被别人换掉」判重用）。
    pub(crate) fn access_token(&self) -> Option<String> {
        self.with_inner(|inner| {
            inner
                .current
                .as_ref()
                .map(|saved| saved.access_token.clone())
        })
    }

    /// 当前 refresh token；未登录 / 为空时为 `None`。
    pub(crate) fn refresh_token(&self) -> Option<String> {
        self.with_inner(|inner| {
            inner
                .current
                .as_ref()
                .map(|saved| saved.refresh_token.clone())
                .filter(|token| !token.is_empty())
        })
    }

    /// 当前令牌是否仍需登录 / 刷新（会话不存在也算「需要」）。
    fn tokens_need_refresh(&self) -> bool {
        self.with_inner(|inner| {
            inner
                .current
                .as_ref()
                .is_none_or(|saved| saved.tokens().needs_refresh(REFRESH_MARGIN))
        })
    }

    fn has_session(&self) -> bool {
        self.with_inner(|inner| inner.current.is_some())
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
        self.close_tree_index();
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
            if !matches!(err, Failure::Auth(_)) {
                return Err(err);
            }
            // 刷新令牌被拒：有记住的密码就静默重登一次，再把原请求继续下去
            match self.silent_relogin() {
                Ok(true) => self.ensure_tokens()?,
                Ok(false) => {
                    self.expire();
                    return Err(err);
                }
                Err(relogin) => {
                    if matches!(relogin, Failure::Auth(_) | Failure::Captcha { .. }) {
                        self.expire();
                    }
                    return Err(relogin);
                }
            }
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

    fn with_tree_index<T>(
        &self,
        f: impl FnOnce(&mut Option<Arc<tree_index::TreeIndex>>) -> T,
    ) -> T {
        let mut guard = match self.tree_index.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        f(&mut guard)
    }

    fn with_tree_index_error<T>(&self, f: impl FnOnce(&mut Option<String>) -> T) -> T {
        let mut guard = match self.tree_index_error.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        f(&mut guard)
    }

    /// 记录一条索引错误；只保留第一条，避免后来的次要失败盖住根因。
    pub(crate) fn record_tree_index_error(&self, message: String) {
        self.with_tree_index_error(|slot| {
            if slot.is_none() {
                *slot = Some(message);
            }
        });
    }

    /// 取走待提示的索引错误（界面提示一次后不再重复）。
    pub(crate) fn take_tree_index_error(&self) -> Option<String> {
        self.with_tree_index_error(Option::take)
    }

    /// 本地索引总开关当前是否打开。
    pub(crate) fn local_index_enabled(&self) -> bool {
        self.index_enabled.load(Ordering::SeqCst)
    }

    /// 运行时切换本地索引。关闭时立即逻辑关闭当前库；关闭后所有读写入口
    /// 都不可见（`tree_index` 返回 `None`），后台线程持有的旧句柄写不进去。
    pub(crate) fn set_local_index_enabled(&self, enabled: bool) {
        self.index_enabled.store(enabled, Ordering::SeqCst);
        if !enabled {
            self.close_tree_index();
        }
    }

    /// 当前账号的索引；未打开、已登出或开关关闭时为 `None`。
    pub(crate) fn tree_index(&self) -> Option<Arc<tree_index::TreeIndex>> {
        if !self.local_index_enabled() {
            return None;
        }
        self.with_tree_index(|slot| slot.clone())
    }

    /// 索引已经成功建过库，可以承担 0 网络的本地读。
    ///
    /// 首次建库尚未完成、索引损坏重建、或已登出时都返回 `false`；
    /// 调用方据此回退到网络路径。
    pub(crate) fn local_index_ready(&self) -> bool {
        let Some(index) = self.tree_index() else {
            return false;
        };
        if index.is_closed() {
            return false;
        }
        index
            .needs_full_build()
            .map(|needed| !needed)
            .unwrap_or(false)
    }

    /// 最近一次全量建库时间（unix 秒）；索引未就绪或读取失败时为 `None`。
    pub(crate) fn index_built_at(&self) -> Option<i64> {
        self.tree_index()?.last_full_build_at().ok().flatten()
    }

    /// 在后台线程写索引：未打开 / 已关闭时跳过，写失败只记录错误。
    ///
    /// 调用点都在网络请求成功之后；这里不向调用方返回错误，写穿失败不能
    /// 把已经成功的用户操作变成失败，也不能覆盖它的成功提示。
    pub(crate) fn write_index(
        &self,
        f: impl FnOnce(&tree_index::TreeIndex) -> Result<(), tree_index::IndexError>,
    ) {
        let Some(index) = self.tree_index() else {
            return;
        };
        if index.is_closed() {
            return;
        }
        if let Err(err) = f(&index) {
            // 登出关闭索引属于正常流程，不当作错误提示
            if !index.is_closed() {
                self.record_tree_index_error(format!("本地索引写入失败：{err}"));
            }
        }
    }

    /// 按当前账号打开索引。旧索引先逻辑关闭，连接交给后台线程自然放掉。
    pub(crate) fn open_tree_index(&self) -> Result<Arc<tree_index::TreeIndex>, Failure> {
        if !self.local_index_enabled() {
            // 开关关闭不是错误：调用方按「索引不可用」静默回退网络
            return Err(Failure::Other("本地索引已关闭".to_owned()));
        }
        let (_, user_id, _) = self.current_credentials()?;
        let path = paths::index_path(&user_id);
        let opened = match tree_index::TreeIndex::open(&path, &user_id) {
            Ok(opened) => opened,
            Err(err) => {
                let message = format!("本地索引不可用：{err}");
                self.record_tree_index_error(message.clone());
                return Err(Failure::Other(message));
            }
        };
        let index = Arc::new(opened);
        if let Some(old) = self.with_tree_index(|slot| slot.replace(Arc::clone(&index))) {
            old.close();
        }
        Ok(index)
    }

    /// 需要时打开：已打开且未关闭就直接复用。
    pub(crate) fn ensure_tree_index_open(&self) -> Result<Arc<tree_index::TreeIndex>, Failure> {
        if let Some(index) = self.tree_index() {
            if !index.is_closed() {
                return Ok(index);
            }
        }
        self.open_tree_index()
    }

    pub(crate) fn close_tree_index(&self) {
        if let Some(index) = self.with_tree_index(Option::take) {
            index.close();
        }
    }

    /// 取当前容量（阻塞）。
    pub fn quota(&self) -> Result<Quota, Failure> {
        let client = self.drive()?;
        client.about().map_err(Failure::from)
    }

    /// 取月度传输配额（阻塞）。
    pub fn transfer_quota(&self) -> Result<TransferQuota, Failure> {
        let client = self.drive()?;
        client.transfer_quota().map_err(Failure::from)
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

    /// 读取全账号回收站（自动翻页；阻塞）。
    pub fn list_trash(&self) -> Result<Vec<TrashEntry>, Failure> {
        let client = self.drive()?;
        let mut entries = Vec::new();
        let mut page_token: Option<String> = None;
        for _ in 0..MAX_LIST_PAGES {
            let (mut page, next) = client
                .list_trash(page_token.as_deref())
                .map_err(Failure::from)?;
            entries.append(&mut page);
            match next {
                Some(token) => page_token = Some(token),
                None => return Ok(entries),
            }
        }
        Err(Failure::Other(format!(
            "回收站条目超过 {} 条，暂不支持完整显示",
            MAX_LIST_PAGES * LIST_PAGE_SIZE
        )))
    }

    /// 下载一条列表缩略图（阻塞）。QML 永远看不到远程地址，只拿到缓存路径。
    pub fn fetch_thumbnail(&self, url: &str) -> Result<Vec<u8>, Failure> {
        let client = self.drive()?;
        client.fetch_thumbnail(url).map_err(Failure::from)
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

    /// 新建文件夹并返回云端 id（上传文件夹批次建目录链用）。
    ///
    /// 调用方先取得并持有 client，保证登出后收尾阶段仍使用同一会话实例。
    pub fn create_folder_entry(
        &self,
        client: Arc<DriveClient>,
        parent_id: Option<&str>,
        name: &str,
        cache: &mut kichi_core::FolderCreateCache,
    ) -> Result<String, Failure> {
        client
            .create_folder_entry_cached(parent_id, name, cache)
            .map_err(Failure::from)
    }

    /// 把普通文件页的条目移入回收站（阻塞）。
    pub fn trash_entries(&self, ids: &[&str]) -> Result<(), Failure> {
        let client = self.drive()?;
        client.trash(ids).map_err(Failure::from)
    }

    /// 从回收站恢复条目（阻塞）。
    pub fn untrash_entries(&self, ids: &[&str]) -> Result<(), Failure> {
        let client = self.drive()?;
        client.untrash(ids).map_err(Failure::from)
    }

    /// 从回收站彻底删除一批条目（阻塞）。调用方负责按 core 上限分批。
    pub fn delete_entries_permanently(&self, ids: &[&str]) -> Result<(), Failure> {
        let client = self.drive()?;
        client.delete_permanently(ids).map_err(Failure::from)
    }

    /// 一次性清空回收站（阻塞，内部等待官方异步任务完成）。
    pub fn clear_trash(&self) -> Result<(), Failure> {
        let client = self.drive()?;
        client.empty_trash().map_err(Failure::from)
    }

    /// 把条目移动到目标文件夹（阻塞）。`None` / 空串表示根目录。
    pub fn move_entries(&self, ids: &[&str], parent_id: Option<&str>) -> Result<(), Failure> {
        let client = self.drive()?;
        client
            .move_to(ids, parent_id.unwrap_or(""))
            .map_err(Failure::from)
    }

    /// 把条目复制到目标文件夹（阻塞）。`None` / 空串表示根目录。
    pub fn copy_entries(&self, ids: &[&str], parent_id: Option<&str>) -> Result<(), Failure> {
        let client = self.drive()?;
        client
            .copy_to(ids, parent_id.unwrap_or(""))
            .map_err(Failure::from)
    }

    /// 给文件或文件夹加星标 / 取消星标（阻塞）。
    pub fn set_starred(&self, ids: &[&str], starred: bool) -> Result<(), Failure> {
        let client = self.drive()?;
        client.set_starred(ids, starred).map_err(Failure::from)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_index_is_never_exposed_or_opened() {
        let session = Session::for_test(Config::from_env(), "token");
        session.set_local_index_enabled(false);

        assert!(!session.local_index_enabled());
        assert!(session.tree_index().is_none(), "关闭后不暴露索引句柄");
        assert!(!session.local_index_ready());
        assert!(session.index_built_at().is_none());
        session.write_index(|_| Ok(()));
        assert!(
            session.take_tree_index_error().is_none(),
            "关闭不是错误，不能攒一条提示"
        );
        assert!(session.open_tree_index().is_err(), "关闭后不得打开库");
        assert!(
            session.take_tree_index_error().is_none(),
            "打开被开关拒绝也不该写错误"
        );
    }

    #[test]
    fn enabling_again_allows_the_index_to_be_used() {
        let session = Session::for_test(Config::from_env(), "token");
        session.set_local_index_enabled(false);
        session.set_local_index_enabled(true);

        assert!(session.local_index_enabled());
        assert!(session.tree_index().is_none(), "只放开开关，不主动开库");
    }

    #[test]
    fn signed_out_session_never_relogs_by_itself() {
        let session = Session::for_test(Config::from_env(), "token");
        session.clear_tokens();
        assert_eq!(session.state(), State::SignedOut);
        // 主动登出后没有会话，静默重登必须直接放弃（不能读钥匙串、不能发请求）
        assert!(!session.silent_relogin().expect("不应发起网络请求"));
    }
}
