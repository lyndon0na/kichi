//! 账号身份：昵称、邮箱、本地头像缓存与上次容量。
//!
//! 昵称和邮箱是两份独立数据；`name` 不再冒充 `email` 写回会话，否则界面上
//! 会把昵称显示成邮箱、也丢失真实昵称。
//!
//! 容量也随会话落盘：启动时先显示上次的值，网络回来再静默校准，
//! 账号区不必每次都从骨架开始。

use std::path::PathBuf;

use kichi_core::Quota;

use super::{avatar_cache, store, Failure, Session};

impl Session {
    pub fn email(&self) -> Option<String> {
        self.with_inner(|inner| {
            inner
                .current
                .as_ref()
                .map(|saved| saved.email.trim().to_owned())
                .filter(|email| !email.is_empty())
        })
    }

    pub fn display_name(&self) -> Option<String> {
        self.with_inner(|inner| {
            inner
                .current
                .as_ref()
                .map(|saved| saved.display_name.trim().to_owned())
                .filter(|name| !name.is_empty())
        })
    }

    pub fn user_id(&self) -> Option<String> {
        self.with_inner(|inner| {
            inner
                .current
                .as_ref()
                .map(|saved| saved.user_id.trim().to_owned())
                .filter(|id| !id.is_empty())
        })
    }

    /// 启动时立即显示上一次成功缓存的头像；不发网络请求。
    pub fn cached_avatar_path(&self) -> Option<PathBuf> {
        self.with_inner(|inner| {
            let saved = inner.current.as_ref()?;
            let user_id = saved.user_id.trim();
            let url = saved.avatar_url.trim();
            if user_id.is_empty() || url.is_empty() {
                return None;
            }
            avatar_cache::find(user_id, url)
        })
    }

    /// 启动时立即显示上一次成功读取的容量；不发网络请求。
    pub fn cached_quota(&self) -> Option<Quota> {
        self.with_inner(|inner| {
            let saved = inner.current.as_ref()?;
            Some(Quota {
                usage: saved.quota_usage?,
                limit: (saved.quota_limit > 0).then_some(saved.quota_limit),
                vip: saved.quota_vip,
            })
        })
    }

    /// 缓存容量到会话存储（内容没变时不重复写盘）。
    pub fn cache_quota(&self, quota: &Quota) {
        let limit = quota.limit.unwrap_or(0);
        let updated = self.with_inner(|inner| {
            let saved = inner.current.as_mut()?;
            if saved.quota_usage == Some(quota.usage)
                && saved.quota_limit == limit
                && saved.quota_vip == quota.vip
            {
                return None;
            }
            saved.quota_usage = Some(quota.usage);
            saved.quota_limit = limit;
            saved.quota_vip = quota.vip;
            Some(saved.clone())
        });
        let Some(saved) = updated else {
            return;
        };
        #[cfg(test)]
        if self.store_disabled {
            return;
        }
        if let Err(err) = store::save(&self.store_path, &saved) {
            eprintln!("[kichi] 容量信息未能落盘：{err}");
        }
    }

    /// 用更完整的身份信息补全会话（`/v1/user/me`，尽力而为）。
    pub fn enrich_identity(&self, name: &str, email: &str, avatar_url: &str) {
        let name = name.trim();
        let email = email.trim();
        let avatar_url = avatar_url.trim();
        let updated = self.with_inner(|inner| {
            let saved = inner.current.as_mut()?;
            let mut changed = false;
            if !name.is_empty() && saved.display_name != name {
                saved.display_name = name.to_owned();
                changed = true;
            }
            if !email.is_empty() && saved.email != email {
                saved.email = email.to_owned();
                changed = true;
            }
            if !avatar_url.is_empty() && saved.avatar_url != avatar_url {
                saved.avatar_url = avatar_url.to_owned();
                changed = true;
            }
            changed.then(|| saved.clone())
        });
        let Some(saved) = updated else {
            return;
        };
        #[cfg(test)]
        if self.store_disabled {
            return;
        }
        if let Err(err) = store::save(&self.store_path, &saved) {
            eprintln!("[kichi] 会话信息更新未能落盘：{err}");
        }
    }

    /// 取当前用户并回填身份（阻塞）。
    pub fn refresh_identity(&self) -> Result<(), Failure> {
        let client = self.drive()?;
        let me = client.me().map_err(Failure::from)?;
        self.enrich_identity(&me.name, &me.email, &me.picture);
        Ok(())
    }

    /// 下载并替换当前账号头像（阻塞）。地址为空时清掉旧头像缓存。
    pub fn refresh_avatar(&self) -> Result<Option<PathBuf>, Failure> {
        let Some((user_id, url)) = self.with_inner(|inner| {
            let saved = inner.current.as_ref()?;
            Some((
                saved.user_id.trim().to_owned(),
                saved.avatar_url.trim().to_owned(),
            ))
        }) else {
            return Ok(None);
        };
        if user_id.is_empty() {
            return Ok(None);
        }
        if url.is_empty() {
            avatar_cache::remove_for_user(&user_id).map_err(Failure::Other)?;
            return Ok(None);
        }
        // 地址就是头像版本：同一账号 + 同一地址已有文件时直接复用。
        // 否则刷新会在 QML 异步读图时删掉并重写同一个路径，造成头像闪一下后
        // 因读取失败退回首字母；换头像时官方 URL 会变化，仍会命中新的缓存键。
        if let Some(path) = avatar_cache::find(&user_id, &url) {
            return Ok(Some(path));
        }
        let client = self.drive()?;
        let bytes = client.fetch_avatar(&url).map_err(Failure::from)?;
        let path = avatar_cache::store(&user_id, &url, &bytes).map_err(Failure::Other)?;
        Ok(Some(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use kichi_core::Config;

    fn session_with(email: &str, display_name: &str, avatar_url: &str) -> Session {
        let saved = super::super::PersistedSession {
            access_token: "access".into(),
            refresh_token: "refresh".into(),
            expires_unix: Utc::now().timestamp() + 3600,
            device_id: "0".repeat(32),
            user_id: "user-1".into(),
            email: email.into(),
            display_name: display_name.into(),
            avatar_url: avatar_url.into(),
            quota_usage: None,
            quota_limit: 0,
            quota_vip: false,
        };
        let device_id = saved.device_id.clone();
        let mut session = Session::new(
            Config::default(),
            std::env::temp_dir().join("kichi-identity-test/session.token"),
            Some(saved),
            device_id,
        );
        session.store_disabled = true;
        session
    }

    #[test]
    fn nickname_never_overwrites_the_email() {
        let session = session_with("old@example.com", "", "");
        session.enrich_identity(
            "夏日用户",
            "new@example.com",
            "https://user.mypikpak.com/avatar/1",
        );
        assert_eq!(session.display_name().as_deref(), Some("夏日用户"));
        assert_eq!(session.email().as_deref(), Some("new@example.com"));
        assert_eq!(
            session
                .with_inner(|inner| inner.current.as_ref().map(|saved| saved.avatar_url.clone())),
            Some("https://user.mypikpak.com/avatar/1".to_owned())
        );
    }

    #[test]
    fn blank_response_fields_do_not_erase_saved_identity() {
        let session = session_with(
            "mail@example.com",
            "旧昵称",
            "https://user.mypikpak.com/avatar/1",
        );
        session.enrich_identity("", "", "");
        assert_eq!(session.display_name().as_deref(), Some("旧昵称"));
        assert_eq!(session.email().as_deref(), Some("mail@example.com"));
    }

    #[test]
    fn old_saved_session_defaults_the_new_identity_fields() {
        let raw = r#"{
            "access_token":"a",
            "refresh_token":"r",
            "expires_unix":1,
            "device_id":"d",
            "user_id":"u",
            "email":"mail@example.com"
        }"#;
        let saved: super::super::PersistedSession =
            serde_json::from_str(raw).expect("旧会话应可反序列化");
        assert_eq!(saved.display_name, "");
        assert_eq!(saved.avatar_url, "");
        assert_eq!(saved.quota_usage, None);
        assert_eq!(saved.quota_limit, 0);
        assert!(!saved.quota_vip);
    }

    #[test]
    fn cached_quota_round_trips_and_keeps_unlimited() {
        let session = session_with("mail@example.com", "", "");
        assert!(session.cached_quota().is_none(), "旧会话没有容量缓存");

        let unlimited = kichi_core::Quota {
            usage: 5,
            limit: None,
            vip: true,
        };
        session.cache_quota(&unlimited);
        let saved = session.cached_quota().expect("缓存后应能读回");
        assert_eq!(saved.usage, 5);
        assert_eq!(saved.limit, None);
        assert!(saved.vip);

        let limited = kichi_core::Quota {
            usage: 12,
            limit: Some(100),
            vip: false,
        };
        session.cache_quota(&limited);
        let saved = session.cached_quota().expect("覆盖后应能读回");
        assert_eq!(saved.usage, 12);
        assert_eq!(saved.limit, Some(100));
        assert!(!saved.vip);
    }
}
