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
    Captcha { url: Option<String> },
    Auth(String),
    Other(String),
}

impl From<CoreError> for Failure {
    fn from(err: CoreError) -> Self {
        match err {
            CoreError::Captcha { url, .. } => Failure::Captcha { url },
            CoreError::Auth { reason } => Failure::Auth(reason),
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

    fn with_inner<T>(&self, f: impl FnOnce(&mut Inner) -> T) -> T {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
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
            Failure::Other(reason) => write!(f, "{reason}"),
        }
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;
    use kichi_core::TaskPhase;

    /// 真实会话的自动刷新验证（默认忽略）。
    ///
    /// 前置条件：先在本机用 GUI 登录一次，令牌已存进 KWallet。
    /// 做法：把内存里的过期时间压到 1 分钟后，`ensure_tokens` 应判定
    /// 「5 分钟内到期」而主动刷新，并用新令牌成功查询容量。
    #[test]
    #[ignore = "需要先在 GUI 登录一次（会话在 KWallet 里）"]
    fn live_auto_refresh_with_stored_session() {
        let path = store_path();
        let loaded = store::load(&path).expect("读取会话存储失败");
        let Some((mut saved, backend)) = loaded else {
            panic!("KWallet 里没有会话，请先在 GUI 登录一次再跑这个测试");
        };
        println!("会话来源：{backend:?}");

        let old_access = saved.access_token.clone();
        // 压到 1 分钟后过期 -> 命中「提前 5 分钟刷新」
        saved.expires_unix = Utc::now().timestamp() + 60;
        let device_id = saved.device_id.clone();
        let config = Config::from_env();
        let session = Session::new(config, path.clone(), Some(saved), device_id);

        session.ensure_tokens().expect("自动刷新失败");

        let refreshed = session.with_inner(|inner| {
            inner
                .current
                .as_ref()
                .map(|s| (s.access_token.clone(), s.expires_unix))
        });
        let Some((new_access, expires_unix)) = refreshed else {
            panic!("刷新后会话丢失");
        };
        assert_ne!(
            old_access, new_access,
            "access_token 没有变化，可能没真正刷新"
        );

        let remaining = expires_unix - Utc::now().timestamp();
        println!("刷新成功，新令牌剩余 {remaining} 秒");
        assert!(remaining > 300, "新令牌有效期异常：{remaining} 秒");

        let quota = session.quota().expect("用新令牌查询容量失败");
        println!(
            "新令牌可用：已用 {} GB，上限 {:?}",
            quota.usage / 1024 / 1024 / 1024,
            quota.limit.map(|l| l / 1024 / 1024 / 1024)
        );

        // 刷新结果应已写回存储：重新加载一次确认
        let reloaded = store::load(&path).expect("重新读取失败");
        let Some((again, _)) = reloaded else {
            panic!("刷新后存储里没有会话");
        };
        assert_eq!(again.access_token, new_access, "刷新结果没有落盘");
        println!("刷新结果已写回存储");
    }

    /// 真实会话的离线任务验证（默认忽略）。
    ///
    /// 前置条件：先在本机用 GUI 登录一次。流程：
    /// 列出任务 → 提交一个公开测试磁力 → 轮询到完成（最多 ~2 分钟）→ 删除任务记录。
    /// 删除时 `delete_files=false`，云端产物保留。
    #[test]
    #[ignore = "需要先在 GUI 登录一次（会话在 KWallet 里）"]
    fn live_offline_task_roundtrip() {
        let path = store_path();
        let loaded = store::load(&path).expect("读取会话存储失败");
        let Some((saved, backend)) = loaded else {
            panic!("KWallet 里没有会话，请先在 GUI 登录一次再跑这个测试");
        };
        println!("会话来源：{backend:?}");

        let device_id = saved.device_id.clone();
        let session = Session::new(Config::from_env(), path, Some(saved), device_id);

        let before = session.offline_tasks().expect("读取离线任务失败");
        println!("现有离线任务 {} 条", before.len());
        for task in before.iter().take(5) {
            println!("- [{:?}] {} ({}%)", task.phase, task.name, task.progress);
        }

        // Blender 官方的 4.4MB 预告片：稳定、体积小，适合验证链路
        const TEST_URL: &str =
            "https://download.blender.org/durian/trailer/sintel_trailer-480p.mp4";
        println!("提交测试任务：{TEST_URL}");
        let created = session
            .add_offline_task(TEST_URL)
            .expect("提交离线任务失败");
        println!(
            "任务已创建：id={} 名称={:?} 阶段={:?}",
            created.id, created.name, created.phase
        );
        assert!(!created.id.is_empty(), "新建任务没有 id");

        // 轮询到该任务不再变化或超时
        let mut final_phase = None;
        for round in 1..=24 {
            std::thread::sleep(std::time::Duration::from_secs(5));
            let tasks = session.offline_tasks().expect("轮询任务列表失败");
            let Some(task) = tasks.iter().find(|t| t.id == created.id) else {
                println!("第 {round} 轮：任务已不在列表中");
                break;
            };
            println!(
                "第 {round} 轮：[{:?}] {} ({}%) {}",
                task.phase,
                task.name,
                task.progress,
                task.message.as_deref().unwrap_or("")
            );
            if matches!(task.phase, TaskPhase::Completed | TaskPhase::Error) {
                final_phase = Some(task.phase);
                break;
            }
        }
        println!("最终阶段：{final_phase:?}");
        assert!(
            final_phase.is_some(),
            "轮询 2 分钟内任务没有进入完成/失败状态"
        );

        session
            .delete_offline_task(&created.id)
            .expect("删除任务失败");
        println!("任务已删除（云端文件保留）");

        let after = session.offline_tasks().expect("删除后再读取失败");
        assert!(
            !after.iter().any(|t| t.id == created.id),
            "删除后任务仍在列表中"
        );
    }

    /// 真实会话的目录列表验证（默认忽略）。
    ///
    /// 前置条件：先在本机用 GUI 登录一次。会读取根目录，并在存在子文件夹时
    /// 进入第一个文件夹再列一次，验证 parent_id 与分页路径。
    #[test]
    #[ignore = "需要先在 GUI 登录一次（会话在 KWallet 里）"]
    fn live_list_root_with_stored_session() {
        let path = store_path();
        let loaded = store::load(&path).expect("读取会话存储失败");
        let Some((saved, backend)) = loaded else {
            panic!("KWallet 里没有会话，请先在 GUI 登录一次再跑这个测试");
        };
        println!("会话来源：{backend:?}");

        let device_id = saved.device_id.clone();
        let session = Session::new(Config::from_env(), path, Some(saved), device_id);
        let entries = session.list_dir(None).expect("读取根目录失败");
        println!("根目录共 {} 条", entries.len());
        for entry in entries.iter().take(10) {
            println!(
                "- [{}] {}（{} 字节，修改于 {:?}）",
                match entry.kind {
                    kichi_core::EntryKind::Folder => "目录",
                    kichi_core::EntryKind::File => "文件",
                },
                entry.name,
                entry.size,
                entry.modified_at
            );
        }
        assert!(!entries.is_empty(), "根目录列表为空，实际使用时应有条目");

        if let Some(folder) = entries
            .iter()
            .find(|e| e.kind == kichi_core::EntryKind::Folder)
        {
            let children = session
                .list_dir(Some(folder.id.as_str()))
                .expect("读取子目录失败");
            println!("子目录「{}」共 {} 条", folder.name, children.len());
        } else {
            println!("根目录没有子文件夹，跳过子目录验证");
        }
    }

    /// 真实会话的播放档位验证（默认忽略）。
    ///
    /// 前置条件：先在本机用 GUI 登录一次。流程：在根目录里找第一个视频/音频
    /// 文件 → 取详情 → 断言至少有一个可播档位 → 对选中的直链发一次探测请求
    /// （只发 Range 头，下载 1 字节），确认官网真的在服务这个链接。
    /// 同时打印 medias 的原始 JSON，核对官方是否下发了字幕字段。
    #[test]
    #[ignore = "需要先在 GUI 登录一次（会话在 KWallet 里）"]
    fn live_play_options_for_a_media_file() {
        let path = store_path();
        let loaded = store::load(&path).expect("读取会话存储失败");
        let Some((saved, backend)) = loaded else {
            panic!("KWallet 里没有会话，请先在 GUI 登录一次再跑这个测试");
        };
        println!("会话来源：{backend:?}");

        let device_id = saved.device_id.clone();
        let session = Session::new(Config::from_env(), path, Some(saved), device_id);

        // 找一个媒体文件：优先根目录，没有再下探一级子目录（最多 3 个）
        let mut found = None;
        let roots = session.list_dir(None).expect("读取根目录失败");
        let mut folders = Vec::new();
        for entry in roots {
            println!(
                "根目录：{} [{}] media={}",
                entry.name,
                if entry.kind == kichi_core::EntryKind::Folder {
                    "目录"
                } else {
                    "文件"
                },
                entry.media
            );
            if entry.media {
                found = Some(entry.clone());
                break;
            }
            if entry.kind == kichi_core::EntryKind::Folder {
                folders.push(entry);
            }
        }
        if found.is_none() {
            for folder in folders.iter().take(3) {
                let Ok(children) = session.list_dir(Some(folder.id.as_str())) else {
                    continue;
                };
                println!("—— 子目录「{}」{} 条 ——", folder.name, children.len());
                for entry in children.iter().take(8) {
                    println!("   {} media={}", entry.name, entry.media);
                }
                if let Some(entry) = children.into_iter().find(|entry| entry.media) {
                    found = Some(entry);
                    break;
                }
            }
        }
        let Some(file) = found else {
            println!("—— 诊断：根目录与子目录都没有 media 标记的条目 ——");
            panic!("没找到媒体文件（video/audio），请先往网盘里放一个视频再跑");
        };
        println!("选中媒体文件：{}（id={}）", file.name, file.id);
        let info = media::file_info(&session, &file.id).expect("取文件详情失败");
        println!(
            "详情：{} 个 medias，links 全空={}",
            info.medias.len(),
            info.all_links_empty
        );
        assert!(!info.all_links_empty, "官方没有下发任何链接");

        let options = info.play_options();
        for option in &options {
            println!(
                "- {:<8} 原画={:<5} 高度={:?} 可用={} 原因={:?} 主机={}",
                option.label,
                option.original,
                option.height,
                option.available,
                option.reason,
                crate::player::mask_url(&option.url)
            );
        }
        let playable: Vec<_> = options.iter().filter(|o| o.playable()).collect();
        assert!(!playable.is_empty(), "没有可播放的档位，界面上会无从播起");
        assert!(
            playable.iter().any(|o| o.original),
            "连原画档都不可用：{}",
            playable.len()
        );

        // 直链真实可用性：只取 1 字节，不下载整个文件（用系统 curl，不给 kichi 加依赖）
        let target = playable
            .iter()
            .find(|o| o.original)
            .copied()
            .or_else(|| playable.first().copied())
            .expect("上面的断言已保证非空");
        let status = std::process::Command::new("curl")
            .args(["-s", "-o", "/dev/null", "-w", "%{http_code}", "-r", "0-0"])
            .arg("--max-time")
            .arg("30")
            .arg(&target.url)
            .output()
            .expect("调用 curl 探测直链失败");
        let code = String::from_utf8_lossy(&status.stdout).trim().to_owned();
        println!(
            "直链探测：HTTP {code}（{}，{}）",
            target.label,
            crate::player::mask_url(&target.url)
        );
        assert!(code == "200" || code == "206", "直链不可用：HTTP {code}");
    }
}
