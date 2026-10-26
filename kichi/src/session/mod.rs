//! 会话：唯一持有令牌、负责登录 / 自动刷新 / 持久化的地方。
#![allow(dead_code)]
//!
//! 纪律（为将来拆 daemon 留路）：QML 只消费本模块产出的数据；
//! 所有阻塞网络调用都经 `run_async` 丢到后台线程，结果回到 Qt 线程。

pub mod media;
mod paths;
mod store;
mod tasks;

pub use paths::store_path;
pub use tasks::has_active;

#[cfg(test)]
mod live_tests;

use std::path::PathBuf;
use std::sync::Mutex;

use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};

use kichi_core::{
    auth, captcha, Config, DriveClient, Error as CoreError, FileEntry, Quota, REFRESH_MARGIN,
};

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

/// 一次操作的结果（给调用方转成界面提示）。
#[derive(Debug)]
pub enum Failure {
    Captcha {
        url: Option<String>,
    },
    Auth(String),
    /// 触发官方限制：配额、日任务数、限速等。不是客户端故障，要给解释而不是重试
    RateLimited {
        detail: String,
    },
    Other(String),
}

impl From<CoreError> for Failure {
    fn from(err: CoreError) -> Self {
        match err {
            CoreError::Captcha { url, .. } => Failure::Captcha { url },
            CoreError::Auth { reason } => Failure::Auth(reason),
            CoreError::RateLimited { reason } => Failure::RateLimited { detail: reason },
            other => Failure::Other(other.to_string()),
        }
    }
}

impl From<StoreError> for Failure {
    fn from(err: StoreError) -> Self {
        Failure::Other(err.to_string())
    }
}

/// 会话门面。
pub struct Session {
    config: Config,
    store_path: PathBuf,
    inner: Mutex<Inner>,
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

    /// 确保令牌可用（到期前 5 分钟自动刷新）。阻塞。
    pub fn ensure_tokens(&self) -> Result<(), Failure> {
        let (refresh_token, needs_refresh) = self.with_inner(|inner| {
            inner
                .current
                .as_ref()
                .map(|s| {
                    (
                        s.refresh_token.clone(),
                        s.tokens().needs_refresh(REFRESH_MARGIN),
                    )
                })
                .ok_or(crate::session::Failure::Auth("尚未登录".into()))
        })?;

        if !needs_refresh {
            return Ok(());
        }

        let tokens = auth::refresh(&self.config, &self.device_id(), &refresh_token)?;
        self.replace_tokens(tokens).map_err(Failure::from)
    }

    fn replace_tokens(&self, tokens: kichi_core::TokenPair) -> Result<(), StoreError> {
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
        store::save(&self.store_path, &saved)?;
        Ok(())
    }

    fn clear_tokens(&self) {
        self.with_inner(|inner| inner.current = None);
    }

    /// 登出：清空内存与持久化。
    pub fn sign_out(&self) -> Result<(), StoreError> {
        store::clear(&self.store_path)?;
        self.clear_tokens();
        Ok(())
    }

    /// 构造网盘客户端（阻塞调用方：可能在后台线程）。令牌失败自动刷新一次。
    pub fn drive(&self) -> Result<DriveClient, Failure> {
        if let Err(err) = self.ensure_tokens() {
            if matches!(err, Failure::Auth(_)) {
                let _ = store::clear(&self.store_path);
                self.clear_tokens();
            }
            return Err(err);
        }
        let current = self.with_inner(|inner| {
            inner.current.as_ref().map(|s| {
                (
                    s.access_token.clone(),
                    s.user_id.clone(),
                    inner.device_id.clone(),
                )
            })
        });
        let (access, user_id, device_id) = current.ok_or(Failure::Auth("尚未登录".into()))?;
        DriveClient::new(&self.config, &device_id, &access, &user_id).map_err(Failure::from)
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
        let client = self.drive()?;
        let mut entries = Vec::new();
        let mut page_token: Option<String> = None;
        for _ in 0..MAX_LIST_PAGES {
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
}

impl Failure {
    /// 是否属于「官方限制」类（界面上要解释原因，不该让用户以为坏了）。
    pub fn is_rate_limited(&self) -> bool {
        matches!(self, Failure::RateLimited { .. })
    }
}

/// 配额 / 限速类错误的完整解释。
///
/// 官方超限一般是**软限速**（约 100KB/s）而不是报错，客户端探测不到；
/// 能收到的都是明确的配额拒绝（日任务数、空间不足、HTTP 429）。
/// 这段文案的目的就是告诉用户「这是官方规则，不是 kichi 出故障」。
pub fn rate_limit_text(failure: &Failure) -> String {
    let detail = match failure {
        Failure::RateLimited { detail } => detail.as_str(),
        _ => "",
    };
    let head = if detail.is_empty() {
        "PikPak 拒绝了这个操作：触发了官方配额或限速限制".to_owned()
    } else {
        format!("PikPak 拒绝了这个操作：{detail}")
    };
    format!(
        "{head}\n这是 PikPak 官方的限制，不是 kichi 故障。\
         免费账号每日总下行约 20GB（新加坡时间 0 点重置），第三方接入合计约 5GB/天；\
         超限后官方会降速到约 100KB/s 而不报错。会员免每日限额，但第三方月度限额仍在。\
         处理办法：等配额重置，或在官方客户端里确认账号状态。"
    )
}

/// 启动恢复的结果。
#[derive(Debug)]
pub struct RestoredState {
    pub state: State,
    pub backend: Option<Backend>,
    pub notice: Option<String>,
    pub email: Option<String>,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failure::Captcha { url: Some(url) } => {
                write!(f, "需要完成人机验证，请在浏览器打开：{url}")
            }
            Failure::Captcha { url: None } => write!(f, "需要完成人机验证"),
            Failure::Auth(reason) => write!(f, "登录状态失效：{reason}"),
            Failure::RateLimited { .. } => write!(f, "{}", rate_limit_text(self)),
            Failure::Other(reason) => write!(f, "{reason}"),
        }
    }
}

#[cfg(test)]
mod limit_tests {
    use super::*;

    #[test]
    fn rate_limit_text_explains_it_is_official_not_a_bug() {
        let failure = Failure::RateLimited {
            detail: "task_daily_create_limit_vip（code 11）".into(),
        };
        assert!(failure.is_rate_limited());
        let text = rate_limit_text(&failure);
        // 用户最需要知道的三件事：不是客户端坏了 / 官方规则是什么 / 怎么办
        assert!(text.contains("不是 kichi 故障"), "{text}");
        assert!(text.contains("20GB"), "要给出具体额度：{text}");
        assert!(text.contains("100KB/s"), "要说明限速表现：{text}");
        assert!(text.contains("code 11"), "保留原始错误码便于排查：{text}");
    }

    #[test]
    fn other_failures_are_not_rate_limited() {
        assert!(!Failure::Other("x".into()).is_rate_limited());
        assert!(!Failure::Auth("x".into()).is_rate_limited());
    }

    #[test]
    fn core_rate_limited_maps_to_the_explained_failure() {
        let failure = Failure::from(kichi_core::Error::RateLimited {
            reason: "file_space_not_enough（code 8）".into(),
        });
        assert!(failure.is_rate_limited());
        assert!(failure.to_string().contains("不是 kichi 故障"));
    }
}
