//! 令牌刷新单飞：并发线程同时发现临期时，只允许一次 refresh 请求。
//!
//! 批量下载从 2026-10-28 起会有多条线程同时跑；它们都会经 [`Session::drive`]
//! 调到这里。服务端的 refresh_token 通常会轮换，并发刷新会出现「后到的覆盖
//! 先到的」，轻则令牌失效、重则掉登录。
//!
//! 做法是**先查、拿锁、再查**的双检：
//! 1. 锁外按 `needs_refresh(REFRESH_MARGIN)` 判断，不用刷就立刻返回；
//! 2. 需要刷才拿 `refresh_lock`；
//! 3. 拿到锁后重新读一次令牌（可能刚被别的线程刷过）；
//! 4. 确实还要刷才发请求。
//!
//! 锁里只有一次阻塞 HTTP；等待刷新完成的线程拿到锁后走第 3 步直接返回。

use kichi_core::{auth, REFRESH_MARGIN};

use super::{Failure, Recovery, Session};

/// 测试注入的刷新实现（生产路径永远走 [`auth::refresh`]）。
#[cfg(test)]
pub(super) type RefreshHook = dyn Fn(&kichi_core::Config, &str, &str) -> Result<kichi_core::TokenPair, kichi_core::Error>
    + Send
    + Sync;

impl Session {
    /// 是否到了该主动刷新的时间（未登录 / 已登出 = false）。
    ///
    /// 界面保活每分钟查一次；真正发请求仍由 [`Session::ensure_tokens`] 单飞。
    pub(crate) fn refresh_due(&self) -> bool {
        self.needs_refresh().unwrap_or(false)
    }

    /// 保活：临期就刷新；刷新被官方拒绝则用记住的密码静默重登。
    ///
    /// 返回 `true` = 走了静默重登，界面需要把登录态切回已登录并刷新身份。
    /// 返回 `false` = 只是提前刷新了令牌（界面无需变化）。
    pub(crate) fn refresh_or_relogin(&self) -> Result<bool, Failure> {
        match self.ensure_tokens() {
            Ok(()) => Ok(false),
            Err(err @ Failure::Auth(_)) => match self.silent_relogin() {
                Ok(true) => Ok(true),
                Ok(false) => {
                    self.expire();
                    Err(err)
                }
                Err(relogin) => {
                    if matches!(relogin, Failure::Auth(_) | Failure::Captcha { .. }) {
                        self.expire();
                    }
                    Err(relogin)
                }
            },
            Err(err) => Err(err),
        }
    }

    /// 服务端明确拒绝了当前令牌（HTTP 401），但本地时钟可能还没到期。
    ///
    /// 先强制刷新一次 refresh token：比整次密码登录轻，也不容易触发人机验证。
    /// 刷新也被拒才用记住的密码重登。
    pub(crate) fn recover_rejected_token(&self) -> Result<Recovery, Failure> {
        let Some(rejected) = self.access_token() else {
            return Ok(Recovery::Unavailable);
        };
        let _guard = self
            .refresh_lock
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        // 等锁期间别的线程可能已经换过令牌：手上这个 401 已经不代表现状了
        if self.access_token().as_deref() != Some(rejected.as_str()) {
            return Ok(Recovery::Refreshed);
        }
        if let Some(refresh_token) = self.refresh_token() {
            match auth::refresh(&self.config, &self.device_id(), &refresh_token) {
                Ok(tokens) => {
                    self.replace_tokens(tokens)?;
                    return Ok(Recovery::Refreshed);
                }
                Err(err) => {
                    let failure = Failure::from(err);
                    // 网络 / 限流不是「令牌被拒」：保持登录态，交给界面提示
                    if !matches!(failure, Failure::Auth(_)) {
                        return Err(failure);
                    }
                }
            }
        }
        let Some((username, password)) = self.remembered_login() else {
            return Ok(Recovery::Unavailable);
        };
        if self.relogin_locked(&username, &password, true)? {
            Ok(Recovery::Relogged)
        } else {
            Ok(Recovery::Unavailable)
        }
    }

    /// 确保令牌可用（到期前 5 分钟自动刷新）。阻塞。
    pub fn ensure_tokens(&self) -> Result<(), Failure> {
        // 快路径：绝大多数调用都不需要刷新，连锁都不用拿
        if !self.needs_refresh()? {
            return Ok(());
        }
        let _guard = self
            .refresh_lock
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        // 等锁期间别的线程可能已经刷完：拿到锁必须重新读一次
        let Some(refresh_token) = self.refresh_token_if_due()? else {
            return Ok(());
        };
        let tokens = self.fetch_refreshed(&refresh_token)?;
        self.replace_tokens(tokens).map_err(Failure::from)
    }

    fn needs_refresh(&self) -> Result<bool, Failure> {
        self.with_inner(|inner| {
            inner
                .current
                .as_ref()
                .map(|saved| saved.tokens().needs_refresh(REFRESH_MARGIN))
                .ok_or_else(|| Failure::Auth("尚未登录".into()))
        })
    }

    /// 锁内复查：仍临期才返回 refresh_token，否则 `None`（已被别人刷过）。
    fn refresh_token_if_due(&self) -> Result<Option<String>, Failure> {
        self.with_inner(|inner| {
            let saved = inner
                .current
                .as_ref()
                .ok_or_else(|| Failure::Auth("尚未登录".into()))?;
            Ok(saved
                .tokens()
                .needs_refresh(REFRESH_MARGIN)
                .then(|| saved.refresh_token.clone()))
        })
    }

    fn fetch_refreshed(&self, refresh_token: &str) -> Result<kichi_core::TokenPair, Failure> {
        #[cfg(test)]
        {
            let hook = self
                .refresh_hook
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone();
            if let Some(hook) = hook {
                return hook(&self.config, &self.device_id(), refresh_token).map_err(Failure::from);
            }
        }
        auth::refresh(&self.config, &self.device_id(), refresh_token).map_err(Failure::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};

    /// 一个 1 分钟后到期的会话：命中 5 分钟提前量。
    fn expiring_session(tag: &str) -> Session {
        let store_path = std::env::temp_dir().join(format!(
            "kichi-refresh-{tag}-{}/session.token",
            std::process::id()
        ));
        let saved = super::super::PersistedSession {
            access_token: "old-access".into(),
            refresh_token: "old-refresh".into(),
            expires_unix: Utc::now().timestamp() + 60,
            device_id: "0".repeat(32),
            user_id: "user-1".into(),
            email: String::new(),
            display_name: String::new(),
            avatar_url: String::new(),
            quota_usage: None,
            quota_limit: 0,
            quota_vip: false,
        };
        let device_id = saved.device_id.clone();
        Session::new(
            kichi_core::Config::default(),
            store_path,
            Some(saved),
            device_id,
        )
    }

    #[test]
    fn eight_threads_refresh_once_and_share_the_new_tokens() {
        let mut session = expiring_session("single-flight");
        // 假刷新令牌绝不能写进真实 KWallet（会覆盖用户的会话）
        session.store_disabled = true;
        let session = Arc::new(session);
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_by_hook = Arc::clone(&calls);
        {
            let mut hook = session
                .refresh_hook
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            *hook = Some(Arc::new(move |_config, _device, refresh| {
                calls_by_hook.fetch_add(1, Ordering::SeqCst);
                // 把窗口拉宽：8 条线程必须都在这期间挤进 ensure_tokens
                std::thread::sleep(std::time::Duration::from_millis(80));
                assert_eq!(refresh, "old-refresh");
                Ok(kichi_core::TokenPair {
                    access_token: "new-access".into(),
                    refresh_token: "new-refresh".into(),
                    expires_at: Utc::now() + Duration::hours(1),
                })
            }));
        }

        let barrier = Arc::new(Barrier::new(8));
        let mut handles = Vec::new();
        for _ in 0..8 {
            let session = Arc::clone(&session);
            let barrier = Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                session.ensure_tokens()
            }));
        }
        for handle in handles {
            handle
                .join()
                .expect("刷新线程正常结束")
                .expect("令牌应刷新成功");
        }

        assert_eq!(calls.load(Ordering::SeqCst), 1, "8 条线程只能发一次刷新");
        let (access, refresh) = session.with_inner(|inner| {
            let saved = inner.current.as_ref().expect("刷新后会话还在");
            (saved.access_token.clone(), saved.refresh_token.clone())
        });
        assert_eq!(access, "new-access");
        assert_eq!(refresh, "new-refresh", "所有线程共享同一份新令牌");
    }

    #[test]
    fn rejected_token_without_a_session_is_not_recoverable() {
        let session = Session::for_test(kichi_core::Config::default(), "access");
        session.clear_tokens();
        assert_eq!(
            session
                .recover_rejected_token()
                .expect("没有会话时不该发请求"),
            Recovery::Unavailable
        );
    }
}
