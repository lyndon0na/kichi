//! 会话级网盘客户端缓存。
//!
//! reqwest 的连接池与 TLS session 都在 Client 里，每次 build 都是新池 + 新握手，
//! 还会起一条专属运行时线程（`docs/pitfalls.md` 第 17 条）。批量下载曾经一个文件
//! 建三个 client，25 个文件实测 1.24 s/个 vs 复用 0.47 s/个。
//!
//! 键是 **access token 本身**：`Session::drive()` 每次都先跑 `ensure_tokens()`，
//! 令牌一换键就不匹配，自然重建——长批次跨令牌刷新也是安全的。
//!
//! 锁里只做 clone / 赋值，不 build（build 会起线程）。

use std::sync::{Arc, Mutex};

use kichi_core::DriveClient;

#[derive(Default)]
pub(super) struct ClientCache {
    cached: Mutex<Option<Cached>>,
}

struct Cached {
    access_token: String,
    client: Arc<DriveClient>,
}

impl ClientCache {
    /// 令牌没变就复用；变了 / 没登录 → `None`。
    pub(super) fn get(&self, access_token: &str) -> Option<Arc<DriveClient>> {
        let cached = self.cached.lock().unwrap_or_else(|e| e.into_inner());
        let entry = cached.as_ref()?;
        (entry.access_token == access_token).then(|| Arc::clone(&entry.client))
    }

    /// 装进缓存，返回实际生效的那个（并发首建时以先到者为准，不产生两个池）。
    ///
    /// 双检是刻意的：先 `get` 再 `install` 的两步之间，别的线程可能已经装好
    /// 同一个令牌的 client；这里以已有者为准，避免两个池同时活着。
    pub(super) fn install(
        &self,
        access_token: String,
        client: Arc<DriveClient>,
    ) -> Arc<DriveClient> {
        let mut cached = self.cached.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = cached.as_ref() {
            if entry.access_token == access_token {
                return Arc::clone(&entry.client);
            }
        }
        *cached = Some(Cached {
            access_token,
            client: Arc::clone(&client),
        });
        client
    }

    /// 登出 / 换令牌 / 换账号时清空。
    pub(super) fn clear(&self) {
        let mut cached = self.cached.lock().unwrap_or_else(|e| e.into_inner());
        *cached = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    /// `DriveClient::new` 不做 I/O（只建 reqwest client），可以直接造。
    fn client(token: &str) -> Arc<DriveClient> {
        let device_id = "0".repeat(32);
        Arc::new(
            DriveClient::new(&kichi_core::Config::default(), &device_id, token, "u")
                .expect("构造客户端（无 I/O）"),
        )
    }

    #[test]
    fn same_token_reuses_the_client() {
        let cache = ClientCache::default();
        let installed = cache.install("t1".into(), client("t1"));
        let got = cache.get("t1").expect("同令牌应命中缓存");
        assert!(Arc::ptr_eq(&installed, &got));
    }

    #[test]
    fn installing_the_same_token_twice_keeps_the_first() {
        let cache = ClientCache::default();
        let first = cache.install("t1".into(), client("t1"));
        let second = cache.install("t1".into(), client("t1"));
        assert!(Arc::ptr_eq(&first, &second), "同一令牌只能有一个池活着");
    }

    #[test]
    fn changing_the_token_drops_the_cache() {
        let cache = ClientCache::default();
        let old = cache.install("t1".into(), client("t1"));
        assert!(cache.get("t2").is_none(), "换了令牌不能命中旧 client");
        let new = cache.install("t2".into(), client("t2"));
        assert!(!Arc::ptr_eq(&old, &new));
        assert!(
            cache.get("t1").is_none(),
            "旧令牌也不能再命中（缓存只留一个）"
        );
        assert!(cache.get("t2").is_some());
    }

    #[test]
    fn concurrent_first_build_ends_with_one_pool() {
        let cache = Arc::new(ClientCache::default());
        let mut handles = Vec::new();
        for _ in 0..8 {
            let cache = Arc::clone(&cache);
            handles.push(std::thread::spawn(move || {
                // 每个线程各造一个：install 必须以先到者为准
                cache.install("t1".into(), client("t1"))
            }));
        }
        let results: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().expect("线程正常结束"))
            .collect();
        let first = &results[0];
        assert!(
            results.iter().all(|client| Arc::ptr_eq(client, first)),
            "8 条线程装同一令牌，只能留下一个 client"
        );
        let cached = cache.get("t1").expect("应命中缓存");
        assert!(Arc::ptr_eq(&cached, first));
    }

    #[test]
    fn clear_forces_a_rebuild() {
        let cache = ClientCache::default();
        cache.install("t1".into(), client("t1"));
        cache.clear();
        assert!(cache.get("t1").is_none(), "清空后不能再命中");
    }

    #[test]
    fn session_drive_reuses_the_client_for_the_same_token() {
        // 接线本身也要有回归：`drive()` 每次都重新 `DriveClient::new` 的话，
        // 批量下载又回到每文件建池（令牌未到期，`ensure_tokens` 不做 I/O）
        let saved = super::super::PersistedSession {
            access_token: "token-1".into(),
            refresh_token: "test-refresh".into(),
            expires_unix: Utc::now().timestamp() + 3600,
            device_id: "0".repeat(32),
            user_id: "user-1".into(),
            email: String::new(),
        };
        let device_id = saved.device_id.clone();
        let session = super::super::Session::new(
            kichi_core::Config::default(),
            std::path::PathBuf::from("/dev/null"),
            Some(saved),
            device_id,
        );
        let first = session.drive().expect("构造网盘客户端失败");
        let second = session.drive().expect("第二次取客户端失败");
        assert!(
            Arc::ptr_eq(&first, &second),
            "同一令牌两次 drive() 必须复用同一个 client"
        );
    }
}
