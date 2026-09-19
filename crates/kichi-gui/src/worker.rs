use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;
use kichi_core::consts::OFFLINE_PHASES;
use kichi_core::download::part_path;
use kichi_core::upload::{OssContext, OssUploadState};
use kichi_core::{session, Error, KichiClient};
use tokio::sync::Notify;

use crate::credentials;
use crate::msg::{Cmd, FolderItem, Msg, QualityOption};

/// 动态并发闸: 上限可随时调整(扩容立刻放行排队任务, 缩容只影响后续
/// acquire、不打断在传任务)。tokio Semaphore 的 set_capacity 未稳定,
/// 而 forget_permits 缩容后会被任务释放的许可回填, 故自行实现。
struct Gate {
    limit: AtomicUsize,
    active: AtomicUsize,
    woken: Notify,
}

/// 占用一个并发槽位的守卫; 释放时唤醒排队任务。
struct GateGuard<'a> {
    gate: &'a Gate,
}

impl Gate {
    fn new(limit: usize) -> Self {
        Self {
            limit: AtomicUsize::new(limit),
            active: AtomicUsize::new(0),
            woken: Notify::new(),
        }
    }

    fn set_limit(&self, limit: usize) {
        self.limit.store(limit.max(1), Ordering::Relaxed);
        self.woken.notify_waiters();
    }

    async fn acquire(&self) -> GateGuard<'_> {
        loop {
            // enable() 先注册唤醒、再判定槽位, 避免判定与挂起之间错过 notify_waiters。
            let notified = self.woken.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let cur = self.active.load(Ordering::Acquire);
            if cur < self.limit.load(Ordering::Relaxed)
                && self
                    .active
                    .compare_exchange_weak(cur, cur + 1, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
            {
                return GateGuard { gate: self };
            }
            notified.await;
        }
    }
}

impl Drop for GateGuard<'_> {
    fn drop(&mut self) {
        self.gate.active.fetch_sub(1, Ordering::AcqRel);
        self.gate.woken.notify_waiters();
    }
}

pub struct Worker {
    pub tx: Sender<Cmd>,
    pub rx: Receiver<Msg>,
}

pub fn spawn() -> Worker {
    let (cmd_tx, cmd_rx) = mpsc::channel::<Cmd>();
    let (msg_tx, msg_rx) = mpsc::channel::<Msg>();
    std::thread::Builder::new()
        .name("kichi-worker".into())
        .spawn(move || run(cmd_rx, msg_tx))
        .expect("failed to spawn worker");
    Worker {
        tx: cmd_tx,
        rx: msg_rx,
    }
}

/// 离线任务每页条数。
const TASK_PAGE_SIZE: usize = 100;
/// 后台空闲轮询基节拍(仅作 recv 超时上限, 命令到达会立即唤醒)。
const POLL_TICK: Duration = Duration::from_millis(800);
/// 离线任务快刷节拍: 存在进行中(等待/下载中)任务时, 需要及时反映状态迁移。
const TASKS_POLL_ACTIVE: Duration = Duration::from_secs(3);
/// 离线任务慢刷节拍: 无进行中任务时, 仅等待新增/外部变更。
const TASKS_POLL_IDLE: Duration = Duration::from_secs(60);
/// 配额轮询节拍: 变化慢, 且登录与删除等操作后已显式刷新。
const QUOTA_POLL: Duration = Duration::from_secs(60);
/// 磁盘缓存淘汰的节流: 距上次扫描超过该间隔就扫一次。
const CACHE_SWEEP_MIN_INTERVAL: Duration = Duration::from_secs(60);
/// 磁盘缓存淘汰的节流: 或累计新增超过该字节数时扫一次。
const CACHE_SWEEP_MIN_NEW_BYTES: u64 = 8 << 20;

/// 磁盘缓存的控制句柄: 「正在使用」登记 + 淘汰节流状态。
///
/// 从 `WorkerState` 里拆出来是为了能按值传进 spawn 出的任务。
#[derive(Clone)]
struct CacheCtl {
    /// 正在写入 / 刚交给系统的缓存路径; 淘汰时跳过。
    in_use: Arc<Mutex<HashSet<PathBuf>>>,
    gate: Arc<Mutex<CacheGate>>,
}

/// 淘汰节流状态: 写入缓存后要累计到一定量或过一段时间, 才真去扫目录。
struct CacheGate {
    last: Instant,
    new_bytes: u64,
}

/// 缓存路径的「正在使用」登记; drop 时自动注销。
struct InUseGuard {
    set: Arc<Mutex<HashSet<PathBuf>>>,
    paths: Vec<PathBuf>,
}

impl Drop for InUseGuard {
    fn drop(&mut self) {
        if let Ok(mut s) = self.set.lock() {
            for p in &self.paths {
                s.remove(p);
            }
        }
    }
}

impl CacheCtl {
    fn new() -> Self {
        Self {
            in_use: Arc::new(Mutex::new(HashSet::new())),
            gate: Arc::new(Mutex::new(CacheGate {
                last: Instant::now(),
                new_bytes: 0,
            })),
        }
    }

    /// 登记一组正在使用的缓存路径(下载目标与 `.part`), 返回的守卫 drop 时注销。
    fn mark_in_use(&self, paths: &[PathBuf]) -> InUseGuard {
        if let Ok(mut s) = self.in_use.lock() {
            s.extend(paths.iter().cloned());
        }
        InUseGuard {
            set: self.in_use.clone(),
            paths: paths.to_vec(),
        }
    }

    /// 记一笔缓存写入(字节数); 达到节流阈值就触发一次后台淘汰。
    fn note_write(&self, bytes: u64) {
        let due = {
            let Ok(mut g) = self.gate.lock() else { return };
            g.new_bytes = g.new_bytes.saturating_add(bytes);
            if g.new_bytes < CACHE_SWEEP_MIN_NEW_BYTES
                && g.last.elapsed() < CACHE_SWEEP_MIN_INTERVAL
            {
                false
            } else {
                g.last = Instant::now();
                g.new_bytes = 0;
                true
            }
        };
        if due {
            tokio::spawn(sweep_caches(self.clone(), false, None));
        }
    }
}

/// 扫描两个缓存根: `purge=false` 按上限淘汰, `purge=true` 清空。
/// `reply` 非空时回传占用情况(设置页展示)。
async fn sweep_caches(cache: CacheCtl, purge: bool, reply: Option<Sender<Msg>>) {
    // 先取一次「正在使用」快照, 供整个扫描过程使用。
    let snapshot: HashSet<PathBuf> = cache.in_use.lock().map(|g| g.clone()).unwrap_or_default();
    let roots: [(PathBuf, crate::cache::Caps); 2] = [
        (preview_root(), crate::cache::PREVIEW_CAPS),
        (thumbnail_cache_dir(), crate::cache::THUMB_CAPS),
    ];

    let result = tokio::task::spawn_blocking(move || {
        let mut total = crate::cache::Sweep::default();
        for (root, caps) in &roots {
            let s = if purge {
                crate::cache::purge(root, &snapshot)
            } else {
                crate::cache::evict_lru(root, caps, &snapshot)
            };
            total.bytes += s.bytes;
            total.entries += s.entries;
            total.removed += s.removed;
            total.freed += s.freed;
        }
        total
    })
    .await;

    match (reply, result) {
        (Some(tx), Ok(total)) => {
            if total.removed > 0 {
                tracing::info!(
                    "缓存清理: 删除 {} 项, 释放 {} 字节",
                    total.removed,
                    total.freed
                );
            }
            let _ = tx.send(Msg::CacheUsage {
                bytes: total.bytes,
                entries: total.entries,
                freed: total.freed,
            });
        }
        (_, Err(e)) => tracing::warn!("缓存扫描任务失败: {e}"),
        _ => {}
    }
}

struct WorkerState {
    client: Option<Arc<KichiClient>>,
    /// 下载/上传取消开关, 按 req_id 索引; 任务结束后自行移除。
    cancel: Arc<tokio::sync::Mutex<HashMap<u64, Arc<AtomicBool>>>>,
    /// 下载并发闸, 超出上限的任务阻塞在 acquire 上排队。
    sem: Arc<Gate>,
    /// 上传并发闸。
    ul_sem: Arc<Gate>,
    /// 单任务最大尝试次数; spawn 新任务时捕获当前值, 进行中任务不受后续修改影响。
    max_attempts: u32,
    /// OSS 分片并发数, 应用于新建 client 与设置变更。
    part_concurrency: usize,
    /// 磁盘缓存(预览 / 缩略图)的登记与淘汰节流。
    cache: CacheCtl,
    /// 已占用的目标文件名集合(键: "目录\0文件名"), 用于同名去重。
    reserved: Arc<Mutex<HashSet<String>>>,
    /// 离线任务每 phase 已加载的页数(刷新时保持分页深度)。
    tasks_pages: BTreeMap<String, usize>,
    /// 离线任务每 phase 的下一页游标(加载更多用); None/缺失表示没有更多。
    tasks_next: BTreeMap<String, Option<String>>,
    /// 上一轮刷新时是否存在进行中(等待/下载中)的离线任务, 决定轮询快慢。
    tasks_active: bool,
    /// 上次刷新离线任务的时间。
    last_tasks_poll: Instant,
    /// 上次刷新配额的时间。
    last_quota_poll: Instant,
}

fn run(rx: Receiver<Cmd>, tx: Sender<Msg>) {
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            let _ = tx.send(Msg::Error {
                what: format!("后台运行时初始化失败: {e}"),
            });
            return;
        }
    };
    rt.block_on(rt_main(rx, tx));
}

async fn rt_main(rx: Receiver<Cmd>, tx: Sender<Msg>) {
    let saved = crate::settings::load();
    let mut st = WorkerState {
        client: None,
        cancel: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        sem: Arc::new(Gate::new(saved.dl_concurrency)),
        ul_sem: Arc::new(Gate::new(saved.ul_concurrency)),
        max_attempts: saved.max_attempts as u32,
        part_concurrency: saved.part_concurrency,
        cache: CacheCtl::new(),
        reserved: Arc::new(Mutex::new(HashSet::new())),
        tasks_pages: BTreeMap::new(),
        tasks_next: BTreeMap::new(),
        tasks_active: false,
        last_tasks_poll: Instant::now(),
        last_quota_poll: Instant::now(),
    };

    tracing::info!("后台 worker 已启动");
    // 启动时后台清一次磁盘缓存: 上次运行可能在写满 / 超限的状态下退出。
    // 丢给 spawn 是为了不挡首屏(自动登录、列目录都排在后面)。
    tokio::spawn(sweep_caches(st.cache.clone(), false, None));
    loop {
        let msg = rx.recv_timeout(POLL_TICK);

        match msg {
            Ok(cmd) => {
                handle(&mut st, &tx, cmd).await;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // 周期性自动刷新: 按「上一次刷新距今」判定, 节拍随任务活动状态自适应。
                if st.client.is_some() {
                    let now = Instant::now();
                    if now.duration_since(st.last_quota_poll) >= QUOTA_POLL {
                        refresh_quota(&mut st, &tx).await;
                    }
                    let tasks_interval = if st.tasks_active {
                        TASKS_POLL_ACTIVE
                    } else {
                        TASKS_POLL_IDLE
                    };
                    if now.duration_since(st.last_tasks_poll) >= tasks_interval {
                        refresh_tasks(&mut st, &tx).await;
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

async fn handle(st: &mut WorkerState, tx: &Sender<Msg>, cmd: Cmd) {
    match cmd {
        Cmd::Login { username, password } => {
            do_login(st, tx, username, password).await;
        }
        Cmd::AutoLogin { username } => {
            // 密钥环读取是阻塞的 D-Bus 调用, 放到阻塞线程池执行。
            let lookup = {
                let username = username.clone();
                tokio::task::spawn_blocking(move || credentials::load(&username))
                    .await
                    .ok()
                    .flatten()
            };
            match lookup {
                Some(password) => do_login(st, tx, username, password).await,
                None => {
                    let _ = tx.send(Msg::AutoLoginUnavailable);
                }
            }
        }
        Cmd::RememberPassword { username, password } => {
            let err = tokio::task::spawn_blocking(move || credentials::save(&username, &password))
                .await
                .ok()
                .and_then(|r| r.err());
            if let Some(what) = err {
                let _ = tx.send(Msg::Error { what });
            }
        }
        Cmd::ForgetPassword { username } => {
            let _ = tokio::task::spawn_blocking(move || credentials::delete(&username)).await;
        }
        Cmd::Resume {
            device_id,
            access_token,
            refresh_token,
            user_id,
            username,
        } => {
            let sess = session::Session {
                device_id: device_id.clone(),
                access_token: access_token.clone(),
                refresh_token: refresh_token.clone(),
                user_id: user_id.clone(),
                username: username.clone(),
            };
            let mut client = KichiClient::new(device_id);
            install_saver(&mut client);
            client.set_part_concurrency(st.part_concurrency);
            client.set_session(&sess).await;
            let client = Arc::new(client);
            tracing::info!("尝试恢复登录态");
            match client.quota().await {
                Ok(_) => {
                    st.client = Some(client);
                    tracing::info!("恢复登录态成功");
                    let _ = tx.send(Msg::LoginOk { username });
                    refresh_quota(st, tx).await;
                    refresh_tasks(st, tx).await;
                }
                // refresh token 已过期/被吊销: 明确要求重新登录。
                Err(Error::AuthExpired(what)) => {
                    tracing::warn!("会话失效(需重新登录): {what}");
                    let _ = tx.send(Msg::SessionInvalid {
                        reason: format!("登录已过期: {what}"),
                    });
                }
                // 只有服务端明确判定凭据失效(API 错误)才强制重新登录;
                // 其余(网络/解析等)先保留本地会话, 由后台周期刷新自动重试。
                Err(e @ Error::Api { .. }) => {
                    tracing::warn!("会话失效: {e}");
                    let _ = tx.send(Msg::SessionInvalid {
                        reason: format!("登录已过期: {e}"),
                    });
                }
                Err(e) => {
                    st.client = Some(client);
                    tracing::warn!("会话暂不可用, 将自动重试: {e}");
                    let _ = tx.send(Msg::LoginOk { username });
                    let _ = tx.send(Msg::Error {
                        what: format!("会话暂不可用, 稍后自动重试: {e}"),
                    });
                }
            }
        }
        Cmd::Logout => {
            // 取消全部下载(含排队中的), 任务自行退出并清理。
            {
                let mut map = st.cancel.lock().await;
                for flag in map.values() {
                    flag.store(true, Ordering::Relaxed);
                }
                map.clear();
            }
            {
                let mut r = st.reserved.lock().unwrap_or_else(|e| e.into_inner());
                r.clear();
            }
            let _ = session::clear_session();
            if let Some(c) = &st.client {
                c.logout().await;
            }
            st.client = None;
            let _ = tx.send(Msg::LoggedOut);
        }
        Cmd::ListFiles {
            parent,
            token,
            append,
            req_id,
        } => {
            let Some(client) = &st.client else { return };
            match client
                .file_list(parent.as_deref(), 100, token.as_deref())
                .await
            {
                Ok(list) => {
                    let _ = tx.send(Msg::Files {
                        parent,
                        req_id,
                        append,
                        list,
                    });
                }
                Err(e) => {
                    let _ = tx.send(Msg::FilesFailed {
                        parent,
                        what: format!("加载文件列表失败: {e}"),
                    });
                }
            }
        }
        Cmd::SearchFiles {
            keyword,
            token,
            append,
            req_id,
        } => {
            let Some(client) = &st.client else {
                return;
            };
            match client.search_files(&keyword, 100, token.as_deref()).await {
                Ok(list) => {
                    tracing::info!("搜索成功: 返回 {} 个结果", list.files.len());
                    let _ = tx.send(Msg::SearchResults {
                        req_id,
                        append,
                        list,
                    });
                }
                Err(e) => {
                    tracing::error!("搜索失败: {}", e);
                    let _ = tx.send(Msg::SearchFailed {
                        what: format!("搜索失败: {e}"),
                    });
                }
            }
        }
        Cmd::CreateFolder { name, parent } => {
            let Some(client) = &st.client else { return };
            match client.create_folder(&name, parent.as_deref()).await {
                Ok(_) => {
                    let _ = tx.send(Msg::FolderCreated);
                }
                Err(e) => {
                    let _ = tx.send(Msg::Error {
                        what: format!("新建文件夹失败: {e}"),
                    });
                }
            }
        }
        Cmd::Rename { id, name } => {
            let Some(client) = &st.client else { return };
            match client.rename(&id, &name).await {
                Ok(_) => {
                    let _ = tx.send(Msg::Renamed);
                }
                Err(e) => {
                    let _ = tx.send(Msg::Error {
                        what: format!("重命名失败: {e}"),
                    });
                }
            }
        }
        Cmd::Trash { ids } => {
            let Some(client) = &st.client else { return };
            match client.batch_trash(&ids).await {
                Ok(_) => {
                    let _ = tx.send(Msg::Trashed);
                }
                Err(e) => {
                    let _ = tx.send(Msg::Error {
                        what: format!("删除失败: {e}"),
                    });
                }
            }
        }
        Cmd::ListTrash {
            token,
            append,
            req_id,
        } => {
            let Some(client) = &st.client else { return };
            match client.trash_list(100, token.as_deref()).await {
                Ok(list) => {
                    let _ = tx.send(Msg::TrashList {
                        req_id,
                        append,
                        list,
                    });
                }
                Err(e) => {
                    let _ = tx.send(Msg::TrashFailed {
                        what: format!("加载回收站失败: {e}"),
                    });
                }
            }
        }
        Cmd::Untrash { ids } => {
            let Some(client) = &st.client else { return };
            match client.batch_untrash(&ids).await {
                Ok(_) => {
                    let _ = tx.send(Msg::TrashRestored { ids });
                }
                Err(e) => {
                    let _ = tx.send(Msg::Error {
                        what: format!("还原失败: {e}"),
                    });
                }
            }
        }
        Cmd::DeleteTrash { ids } => {
            let Some(client) = &st.client else { return };
            match client.batch_delete(&ids).await {
                Ok(_) => {
                    let _ = tx.send(Msg::TrashDeleted { ids });
                }
                Err(e) => {
                    let _ = tx.send(Msg::Error {
                        what: format!("彻底删除失败: {e}"),
                    });
                }
            }
        }
        Cmd::EmptyTrash => {
            let Some(client) = &st.client else { return };
            match client.empty_trash().await {
                Ok(()) => {
                    let _ = tx.send(Msg::TrashEmptied);
                }
                Err(e) => {
                    let _ = tx.send(Msg::Error {
                        what: format!("清空回收站失败: {e}"),
                    });
                }
            }
        }
        Cmd::MoveTo { ids, dest, src } => {
            let Some(client) = &st.client else { return };
            match client.batch_move(&ids, dest.as_deref()).await {
                Ok(_) => {
                    let _ = tx.send(Msg::Moved { ids, src, dest });
                }
                Err(e) => {
                    let _ = tx.send(Msg::Error {
                        what: format!("移动失败: {e}"),
                    });
                }
            }
        }
        Cmd::CopyTo { ids, dest } => {
            let Some(client) = &st.client else { return };
            match client.batch_copy(&ids, dest.as_deref()).await {
                Ok(_) => {
                    let _ = tx.send(Msg::Copied { dest });
                }
                Err(e) => {
                    let _ = tx.send(Msg::Error {
                        what: format!("复制失败: {e}"),
                    });
                }
            }
        }
        Cmd::OfflineCreate { url, name, parent } => {
            let Some(client) = &st.client else { return };
            match client
                .offline_create(&url, name.as_deref(), parent.as_deref())
                .await
            {
                Ok(_) => {
                    let _ = tx.send(Msg::OfflineCreated);
                }
                Err(e) => {
                    let _ = tx.send(Msg::Error {
                        what: format!("添加离线下载失败: {e}"),
                    });
                }
            }
        }
        Cmd::ListFolders { parent, req_id } => {
            let Some(client) = &st.client else { return };
            match client.file_list(parent.as_deref(), 100, None).await {
                Ok(list) => {
                    let files: Vec<_> = list.files.into_iter().filter(|f| f.is_folder()).collect();
                    let _ = tx.send(Msg::Folders {
                        parent,
                        req_id,
                        files,
                    });
                }
                Err(e) => {
                    let _ = tx.send(Msg::Folders {
                        parent: parent.clone(),
                        req_id,
                        files: Vec::new(),
                    });
                    let _ = tx.send(Msg::Error {
                        what: format!("加载目录失败: {e}"),
                    });
                }
            }
        }
        Cmd::OfflineRetry { task_id } => {
            let Some(client) = &st.client else { return };
            match client.offline_retry(&task_id).await {
                Ok(_) => {
                    let _ = tx.send(Msg::OfflineRetried);
                }
                Err(e) => {
                    let _ = tx.send(Msg::Error {
                        what: format!("重试任务失败: {e}"),
                    });
                }
            }
        }
        Cmd::OfflineDelete {
            task_ids,
            delete_files,
        } => {
            let Some(client) = &st.client else { return };
            match client.offline_delete(&task_ids, delete_files).await {
                Ok(_) => {
                    let _ = tx.send(Msg::OfflineDeleted);
                }
                Err(e) => {
                    let _ = tx.send(Msg::Error {
                        what: format!("删除任务失败: {e}"),
                    });
                }
            }
        }
        Cmd::RefreshTasks => refresh_tasks(st, tx).await,
        Cmd::LoadMoreTasks { phase } => {
            let Some(client) = st.client.clone() else {
                return;
            };
            let Some(token) = st.tasks_next.get(&phase).cloned().flatten() else {
                return;
            };
            // 手动翻页也算一次「刚拉过」, 避免紧接着又触发自动刷新。
            st.last_tasks_poll = Instant::now();
            match client
                .offline_list_phase(&phase, TASK_PAGE_SIZE, Some(&token))
                .await
            {
                Ok(page) => {
                    *st.tasks_pages.entry(phase.clone()).or_insert(1) += 1;
                    st.tasks_next
                        .insert(phase.clone(), page.next_page_token.clone());
                    let _ = tx.send(Msg::TasksMore {
                        phase,
                        tasks: page.tasks,
                        next_page_token: page.next_page_token,
                    });
                }
                Err(e) => {
                    let _ = tx.send(Msg::TasksMoreFailed {
                        phase,
                        what: format!("加载更多任务失败: {e}"),
                    });
                }
            }
        }
        Cmd::RefreshQuota => refresh_quota(st, tx).await,
        Cmd::StartDownload {
            req_id,
            file_id,
            name,
            dest_dir,
        } => {
            spawn_download(st, tx, req_id, file_id, name, dest_dir).await;
        }
        Cmd::StartDownloadFolder {
            req_id,
            folder_id,
            name,
            dest_dir,
        } => {
            spawn_folder_download(st, tx, req_id, folder_id, name, dest_dir).await;
        }
        Cmd::CancelDownload { req_id } => {
            let map = st.cancel.lock().await;
            if let Some(flag) = map.get(&req_id) {
                flag.store(true, Ordering::Relaxed);
            }
        }
        Cmd::StartUpload {
            req_id,
            path,
            parent,
        } => {
            spawn_upload(st, tx, req_id, path, parent).await;
        }
        Cmd::StartUploadDir {
            req_id,
            path,
            parent,
        } => {
            spawn_upload_dir(st, tx, req_id, path, parent).await;
        }
        Cmd::CancelUpload { req_id } => {
            let map = st.cancel.lock().await;
            if let Some(flag) = map.get(&req_id) {
                flag.store(true, Ordering::Relaxed);
            }
        }
        Cmd::Preview {
            req_id,
            file_id,
            name,
            media,
            subtitles,
        } => {
            let Some(client) = st.client.clone() else {
                return;
            };
            let tx = tx.clone();
            let cache = st.cache.clone();
            tokio::spawn(async move {
                if media {
                    preview_stream(&client, &tx, req_id, file_id, name, subtitles, cache).await;
                } else {
                    preview_download(&client, &tx, req_id, file_id, name, cache).await;
                }
            });
        }
        Cmd::PreviewQualities { file_id, subtitles } => {
            let Some(client) = st.client.clone() else {
                return;
            };
            let tx = tx.clone();
            let cache = st.cache.clone();
            tokio::spawn(async move {
                preview_qualities(&client, &tx, file_id, subtitles, cache).await;
            });
        }
        Cmd::CreateShare {
            file_ids,
            expiration_days,
            need_password,
            label,
        } => {
            let Some(client) = &st.client else { return };
            match client
                .share_create(&file_ids, expiration_days, need_password)
                .await
            {
                Ok(c) => {
                    let _ = tx.send(Msg::ShareCreated {
                        url: c.share_url,
                        pass_code: c.pass_code,
                        share_text: c.share_text,
                        label,
                    });
                }
                Err(e) => {
                    let _ = tx.send(Msg::Error {
                        what: format!("创建分享失败: {e}"),
                    });
                }
            }
        }
        Cmd::ListShares {
            token,
            append,
            req_id,
        } => {
            let Some(client) = &st.client else { return };
            match client.share_list(100, token.as_deref()).await {
                Ok(list) => {
                    let _ = tx.send(Msg::Shares {
                        req_id,
                        append,
                        list,
                    });
                }
                Err(e) => {
                    let _ = tx.send(Msg::SharesFailed {
                        what: format!("加载分享列表失败: {e}"),
                    });
                }
            }
        }
        Cmd::DeleteShares { ids } => {
            let Some(client) = &st.client else { return };
            match client.share_batch_delete(&ids).await {
                Ok(()) => {
                    let _ = tx.send(Msg::SharesDeleted { ids });
                }
                Err(e) => {
                    let _ = tx.send(Msg::Error {
                        what: format!("取消分享失败: {e}"),
                    });
                }
            }
        }
        Cmd::ResolveShare {
            share_id,
            pass_code,
        } => {
            let Some(client) = &st.client else { return };
            match client.share_info(&share_id, &pass_code).await {
                Ok(detail) => {
                    let _ = tx.send(Msg::ShareResolved {
                        share_id,
                        title: detail.title,
                        pass_code_token: detail.pass_code_token,
                        files: detail.files,
                        next_page_token: detail.next_page_token,
                    });
                }
                Err(e) => {
                    let _ = tx.send(Msg::ShareResolveFailed {
                        what: format!("解析分享失败: {e}"),
                    });
                }
            }
        }
        Cmd::LoadMoreShareFiles {
            share_id,
            pass_code_token,
            page_token,
        } => {
            let Some(client) = &st.client else { return };
            match client
                .share_detail(&share_id, &pass_code_token, &page_token)
                .await
            {
                Ok(detail) => {
                    let _ = tx.send(Msg::ShareFilesLoaded {
                        files: detail.files,
                        next_page_token: detail.next_page_token,
                    });
                }
                Err(e) => {
                    let _ = tx.send(Msg::ShareFilesLoadFailed {
                        what: format!("加载更多文件失败: {e}"),
                    });
                }
            }
        }
        Cmd::SaveShare {
            share_id,
            pass_code_token,
            file_ids,
            dest,
        } => {
            let Some(client) = &st.client else { return };

            // 若用户指定了目标目录, 先快照「转存自分享」现有内容
            let before_ids: Option<HashSet<String>> = if dest.is_some() {
                snapshot_pack_folder(client).await.ok()
            } else {
                None
            };

            match client
                .share_restore(&share_id, &pass_code_token, &file_ids)
                .await
            {
                Ok(_) => {
                    // 若用户指定了目标目录, 等转存完成后只移动新增的文件
                    if let Some(dest_id) = dest {
                        if let Err(e) =
                            move_new_files(client, &dest_id, before_ids.unwrap_or_default()).await
                        {
                            tracing::warn!("自动移动转存文件失败: {e}");
                            let _ = tx.send(Msg::ShareSaved {
                                auto_move_failed: true,
                            });
                            return;
                        }
                    }
                    let _ = tx.send(Msg::ShareSaved {
                        auto_move_failed: false,
                    });
                }
                Err(e) => {
                    let _ = tx.send(Msg::ShareSaveFailed {
                        what: format!("转存失败: {e}"),
                    });
                }
            }
        }
        Cmd::RetryMoveShare { dest } => {
            let Some(client) = &st.client else { return };
            // 找到「转存自分享」文件夹
            let folder = match find_pack_folder(client).await {
                Ok(Some(f)) => f,
                Ok(None) => {
                    let _ = tx.send(Msg::ShareMoveRetryFailed {
                        what: "未找到「转存自分享」文件夹".to_string(),
                    });
                    return;
                }
                Err(e) => {
                    let _ = tx.send(Msg::ShareMoveRetryFailed {
                        what: format!("加载文件列表失败: {e}"),
                    });
                    return;
                }
            };
            // 获取文件夹中的所有文件
            let pack_list = match client.file_list(Some(&folder.id), 100, None).await {
                Ok(list) => list,
                Err(e) => {
                    let _ = tx.send(Msg::ShareMoveRetryFailed {
                        what: format!("加载转存文件失败: {e}"),
                    });
                    return;
                }
            };
            if pack_list.files.is_empty() {
                let _ = tx.send(Msg::ShareMoveRetryFailed {
                    what: "「转存自分享」中没有文件".to_string(),
                });
                return;
            }
            let file_ids: Vec<String> = pack_list.files.iter().map(|f| f.id.clone()).collect();
            match client.batch_move(&file_ids, Some(&dest)).await {
                Ok(_) => {
                    let _ = tx.send(Msg::ShareMoveRetried);
                }
                Err(e) => {
                    let _ = tx.send(Msg::ShareMoveRetryFailed {
                        what: format!("移动失败: {e}"),
                    });
                }
            }
        }
        Cmd::LoadThumbnail { file_id, url } => {
            load_thumbnail(st, tx, file_id, url).await;
        }
        Cmd::SetTransferLimits {
            dl_concurrency,
            ul_concurrency,
            part_concurrency,
            max_attempts,
        } => {
            // 并发即时生效: 扩容立刻放行排队任务, 缩容只影响后续 acquire, 不打断在传任务。
            st.sem.set_limit(dl_concurrency);
            st.ul_sem.set_limit(ul_concurrency);
            st.max_attempts = max_attempts as u32;
            st.part_concurrency = part_concurrency;
            if let Some(client) = &st.client {
                client.set_part_concurrency(part_concurrency);
            }
        }
        Cmd::MaintainCache { purge } => {
            // 扫描 / 删除都是阻塞 IO, 交给后台任务, 不占住 worker 主循环。
            let cache = st.cache.clone();
            let tx = tx.clone();
            tokio::spawn(async move { sweep_caches(cache, purge, Some(tx)).await });
        }
    }
}

/// 定位服务端转存暂存目录(「转存自分享」/ "Pack From Shared")。
/// 优先使用持久化的目录 ID; ID 失效(如用户删除后服务端重建)时回退到名称匹配,
/// 并把新 ID 写回缓存。服务端从未产生过该目录时返回 None。
async fn find_pack_folder(client: &KichiClient) -> Result<Option<kichi_core::types::File>, Error> {
    let cached = crate::settings::load_pack_folder_id();
    let root_list = client.file_list(None, 100, None).await?;
    let found = root_list
        .files
        .iter()
        .find(|f| f.is_folder() && Some(&f.id) == cached.as_ref())
        .or_else(|| {
            root_list.files.iter().find(|f| {
                f.is_folder()
                    && (f.name.contains("Pack From Shared") || f.name.contains("转存自分享"))
            })
        });
    match found {
        Some(f) => {
            if Some(&f.id) != cached.as_ref() {
                crate::settings::save_pack_folder_id(&f.id);
            }
            Ok(Some(f.clone()))
        }
        None => Ok(None),
    }
}

/// 快照「转存自分享」文件夹中现有的文件 id 集合。
async fn snapshot_pack_folder(client: &KichiClient) -> Result<HashSet<String>, Error> {
    let Some(folder) = find_pack_folder(client).await? else {
        return Ok(HashSet::new());
    };
    let pack_list = client.file_list(Some(&folder.id), 100, None).await?;
    Ok(pack_list.files.iter().map(|f| f.id.clone()).collect())
}

/// 转存后自动移动: 等待服务端写入完成, 找出「转存自分享」中新增的文件并移动到目标目录。
/// 使用重试机制轮询等待服务端同步, 而非固定 sleep。
async fn move_new_files(
    client: &KichiClient,
    dest_id: &str,
    before_ids: HashSet<String>,
) -> Result<(), Error> {
    // 重试机制: 最多轮询 5 次, 间隔递增 (1s, 2s, 3s, 4s, 5s)
    let max_attempts = 5;
    let mut new_ids: Vec<String> = Vec::new();

    for attempt in 0..max_attempts {
        // 等待让服务端完成转存写入
        tokio::time::sleep(Duration::from_secs(1 + attempt as u64)).await;

        let Some(folder) = find_pack_folder(client).await? else {
            if attempt == max_attempts - 1 {
                return Err(Error::msg("未找到「转存自分享」文件夹"));
            }
            continue;
        };

        let pack_list = client.file_list(Some(&folder.id), 100, None).await?;
        new_ids = pack_list
            .files
            .iter()
            .filter(|f| !before_ids.contains(&f.id))
            .map(|f| f.id.clone())
            .collect();

        // 如果找到新文件, 跳出重试循环
        if !new_ids.is_empty() {
            break;
        }

        tracing::debug!("自动移动: 第 {} 次轮询未发现新文件", attempt + 1);
    }

    if new_ids.is_empty() {
        return Ok(());
    }

    client.batch_move(&new_ids, Some(dest_id)).await?;
    Ok(())
}

/// 账号密码登录的公共实现(手动登录与密钥环自动登录共用)。
async fn do_login(st: &mut WorkerState, tx: &Sender<Msg>, username: String, password: String) {
    let device_id = kichi_core::captcha::generate_device_id();
    let mut client = KichiClient::new(device_id.clone());
    install_saver(&mut client);
    client.set_part_concurrency(st.part_concurrency);
    tracing::info!("开始登录: {username}");
    match client.login(&username, &password).await {
        Ok(sess) => {
            if let Err(e) = session::save_session(&sess) {
                let _ = tx.send(Msg::Error {
                    what: e.to_string(),
                });
            }
            st.client = Some(Arc::new(client));
            tracing::info!("登录成功: {username}");
            let _ = tx.send(Msg::LoginOk { username });
            refresh_quota(st, tx).await;
            refresh_tasks(st, tx).await;
        }
        Err(e) => {
            tracing::warn!("登录失败: {e}");
            let (what, verify_url) = match &e {
                Error::CaptchaReview { url, description } => {
                    (format!("需要人机验证: {description}"), url.clone())
                }
                other => (format!("登录失败: {other}"), None),
            };
            let _ = tx.send(Msg::LoginFailed { what, verify_url });
        }
    }
}

fn install_saver(client: &mut KichiClient) {
    let saver: kichi_core::client::TokenSaver = Arc::new(|sess| {
        let _ = session::save_session(sess);
    });
    client.set_token_saver(saver);
}

// ---------------- 文件预览 ----------------

/// 预览缓存目录: `~/.cache/kichi/preview`(取不到时退回临时目录)。
fn preview_root() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("kichi")
        .join("preview")
}

/// 某文件的本地缓存路径: `<cache>/<file_id>/<文件名>`。
/// 同一文件重复预览时可直接命中缓存, 不再下载。
fn preview_cache_path(file_id: &str, name: &str) -> PathBuf {
    let dir: String = file_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    let dir = if dir.is_empty() {
        "unknown".to_string()
    } else {
        dir
    };
    let safe = crate::format::safe_file_name(name).unwrap_or_else(|| "preview".to_string());
    preview_root().join(dir).join(safe)
}

/// 媒体预览: 解析限时直链后交给 UI, 由外部播放器流式播放。
/// 同集外挂字幕会先下载到本地缓存, 一并交给播放器挂载。
async fn preview_stream(
    client: &KichiClient,
    tx: &Sender<Msg>,
    req_id: u64,
    file_id: String,
    name: String,
    subtitles: Vec<(String, String)>,
    cache: CacheCtl,
) {
    let subs = prepare_subtitles(client, &subtitles, &cache).await;
    match client.file_download_link(&file_id).await {
        Ok(link) => {
            let headers = client.stream_headers(&link.url).await;
            let _ = tx.send(Msg::PreviewStream {
                req_id,
                name,
                url: link.url,
                headers,
                subs,
            });
        }
        Err(e) => {
            let _ = tx.send(Msg::PreviewFailed {
                req_id,
                what: format!("解析播放地址失败: {e}"),
            });
        }
    }
}

/// 解析媒体文件的可用清晰度列表, 连同同集字幕一起交给 UI 供选择。
async fn preview_qualities(
    client: &KichiClient,
    tx: &Sender<Msg>,
    file_id: String,
    subtitles: Vec<(String, String)>,
    cache: CacheCtl,
) {
    let subs = prepare_subtitles(client, &subtitles, &cache).await;
    match client.media_variants(&file_id).await {
        Ok(variants) => {
            // 每个清晰度单独探测所需请求头(通常 2~4 项)。
            let mut qualities = Vec::with_capacity(variants.len());
            for v in variants {
                let headers = client.stream_headers(&v.url).await;
                qualities.push(QualityOption {
                    label: v.label,
                    url: v.url,
                    headers,
                });
            }
            let _ = tx.send(Msg::PreviewQualities {
                file_id,
                qualities,
                subs,
            });
        }
        Err(e) => {
            let _ = tx.send(Msg::QualitiesFailed {
                file_id,
                what: format!("获取清晰度失败: {e}"),
            });
        }
    }
}

/// 下载同集外挂字幕到预览缓存, 返回本地路径。
/// 尽力而为: 单条失败(解析直链或下载出错)时跳过, 不影响视频播放。
async fn prepare_subtitles(
    client: &KichiClient,
    subtitles: &[(String, String)],
    cache: &CacheCtl,
) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for (id, name) in subtitles {
        let dest = preview_cache_path(id, name);
        if dest.exists() {
            // 命中缓存: 刷新 mtime, 让 LRU 知道它刚被用过。
            crate::cache::touch(&dest);
            paths.push(dest);
            continue;
        }
        let Ok(link) = client.file_download_link(id).await else {
            continue;
        };
        // 下载期间(含 `.part`)登记为使用中, 避免被并发的淘汰删掉。
        let guard = cache.mark_in_use(&[dest.clone(), part_path(&dest)]);
        match client.download_to(&link, &dest, None, |_, _| {}).await {
            Ok(size) => {
                drop(guard);
                cache.note_write(size);
                paths.push(dest);
            }
            Err(e) => tracing::debug!("字幕下载失败 {name}: {e}"),
        }
    }
    paths
}

/// 非媒体预览: 下载到本地缓存(命中缓存则跳过), 再交给系统查看器打开。
async fn preview_download(
    client: &KichiClient,
    tx: &Sender<Msg>,
    req_id: u64,
    file_id: String,
    name: String,
    cache: CacheCtl,
) {
    let dest = preview_cache_path(&file_id, &name);
    if dest.exists() {
        // 命中缓存: 刷新 mtime, 让 LRU 知道它刚被用过。
        crate::cache::touch(&dest);
        let _ = tx.send(Msg::PreviewReady {
            req_id,
            name,
            path: dest,
        });
        return;
    }
    let link = match client.file_download_link(&file_id).await {
        Ok(l) => l,
        Err(e) => {
            let _ = tx.send(Msg::PreviewFailed {
                req_id,
                what: format!("解析下载地址失败: {e}"),
            });
            return;
        }
    };
    // 下载期间(含 `.part`)登记为使用中, 避免被并发的淘汰删掉。
    let guard = cache.mark_in_use(&[dest.clone(), part_path(&dest)]);
    match client.download_to(&link, &dest, None, |_, _| {}).await {
        Ok(size) => {
            drop(guard);
            cache.note_write(size);
            let _ = tx.send(Msg::PreviewReady {
                req_id,
                name,
                path: dest,
            });
        }
        Err(e) => {
            let _ = tx.send(Msg::PreviewFailed {
                req_id,
                what: format!("准备预览文件失败: {e}"),
            });
        }
    }
}

// ---------------- 缩略图 ----------------

/// 缩略图缓存目录: `~/.cache/kichi/thumbnails`。
fn thumbnail_cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("kichi")
        .join("thumbnails")
}

/// 某文件缩略图的本地缓存路径: `<cache>/<file_id>.jpg`。
fn thumbnail_cache_path(file_id: &str) -> PathBuf {
    let safe: String = file_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    let safe = if safe.is_empty() {
        "unknown".to_string()
    } else {
        safe
    };
    thumbnail_cache_dir().join(format!("{safe}.jpg"))
}

/// 加载缩略图: 先检查磁盘缓存, 未命中则从 URL 下载, 解码为 RGBA 后发送给 UI。
async fn load_thumbnail(st: &WorkerState, tx: &Sender<Msg>, file_id: String, url: String) {
    let Some(client) = st.client.clone() else {
        return;
    };
    let dest = thumbnail_cache_path(&file_id);

    // 磁盘缓存未命中时下载; 写入期间登记为使用中(缩略图是「写完即读」)。
    if !dest.exists() {
        let guard = st.cache.mark_in_use(std::slice::from_ref(&dest));
        if let Err(e) = client.download_thumbnail(&url, &dest).await {
            tracing::debug!("缩略图下载失败 {}: {e}", file_id);
            return;
        }
        let size = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
        drop(guard);
        st.cache.note_write(size);
    } else {
        // 命中缓存: 刷新 mtime, 让 LRU 知道它刚被用过。
        crate::cache::touch(&dest);
    }

    // 从磁盘读取并解码。
    let file_id_clone = file_id.clone();
    let read_path = dest.clone();
    let result = tokio::task::spawn_blocking(move || -> Option<(u32, u32, Vec<egui::Color32>)> {
        let data = std::fs::read(&read_path).ok()?;
        let img = image::load_from_memory(&data).ok()?;
        let rgba = img.to_rgba8();
        let (w, h) = rgba.dimensions();
        let pixels: Vec<egui::Color32> = rgba
            .pixels()
            .map(|p| egui::Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3]))
            .collect();
        Some((w, h, pixels))
    })
    .await;

    match result {
        Ok(Some((w, h, pixels))) => {
            let _ = tx.send(Msg::ThumbnailReady {
                file_id: file_id_clone,
                width: w,
                height: h,
                pixels,
            });
        }
        Ok(None) => {
            tracing::debug!("缩略图解码失败 {}", file_id_clone);
        }
        Err(e) => {
            tracing::debug!("缩略图解码任务失败 {}: {e}", file_id_clone);
        }
    }
}

// ---------------- 本地下载调度 ----------------

/// 注册一个下载任务并 spawn 后台协程。目标文件名在此处净化、去重并占位,
/// 因此即便任务在并发信号量上排队, 同名任务也会分到不同的文件。
async fn spawn_download(
    st: &WorkerState,
    tx: &Sender<Msg>,
    req_id: u64,
    file_id: String,
    name: String,
    dest_dir: PathBuf,
) {
    let Some(client) = st.client.clone() else {
        return;
    };

    let base = crate::format::safe_file_name(&name).unwrap_or_else(|| "download".to_string());
    // 在注册表(避免同名并发)与磁盘(避免覆盖已有文件)中都不冲突时才占用。
    let dest = {
        let mut reserved = st.reserved.lock().unwrap_or_else(|e| e.into_inner());
        let mut i: u32 = 0;
        loop {
            let cand_name = unique_name(&base, i);
            let cand = dest_dir.join(&cand_name);
            let key = reserve_key(&dest_dir, &cand_name);
            if !reserved.contains(&key) && !cand.exists() {
                reserved.insert(key);
                break cand;
            }
            i += 1;
        }
    };

    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut map = st.cancel.lock().await;
        map.insert(req_id, cancel.clone());
    }
    let cancel_map = st.cancel.clone();
    let sem = st.sem.clone();
    let max_attempts = st.max_attempts;
    let reserved = st.reserved.clone();
    let msg_tx = tx.clone();

    tokio::spawn(async move {
        // 抢占并发槽位; 若等待期间被取消则直接退出(释放占位)。
        let permit = sem.acquire().await;
        if cancel.load(Ordering::Relaxed) {
            cleanup_dl(&cancel_map, &reserved, req_id, &dest).await;
            discard_part(&dest);
            drop(permit);
            let _ = msg_tx.send(Msg::DlCancelled { req_id });
            return;
        }
        // 先推一条 0 进度, 让 UI 从"排队中"进入"运行/解析中"。
        let _ = msg_tx.send(Msg::DlProgress {
            req_id,
            total: 0,
            done: 0,
        });
        let outcome = run_download(
            &client,
            &msg_tx,
            req_id,
            file_id,
            dest.clone(),
            cancel.clone(),
            max_attempts,
        )
        .await;
        cleanup_dl(&cancel_map, &reserved, req_id, &dest).await;
        drop(permit);

        match outcome {
            Ok(bytes) => {
                tracing::info!("下载完成 req={req_id} ({bytes} 字节)");
                let _ = msg_tx.send(Msg::DlFinished { req_id, bytes });
            }
            Err(e) => {
                if cancel.load(Ordering::Relaxed) {
                    // 取消: 未完成的 .part 直接删除, 不保留续传文件。
                    discard_part(&dest);
                    let _ = msg_tx.send(Msg::DlCancelled { req_id });
                } else {
                    tracing::warn!("下载失败 req={req_id}: {e}");
                    let _ = msg_tx.send(Msg::DlFailed {
                        req_id,
                        what: e.to_string(),
                    });
                }
            }
        }
    });
}

/// 扫描云端目录树, 在本地按层级建目录, 再把文件清单回传 UI 逐个下载。
async fn spawn_folder_download(
    st: &WorkerState,
    tx: &Sender<Msg>,
    req_id: u64,
    folder_id: String,
    name: String,
    dest_dir: PathBuf,
) {
    let Some(client) = st.client.clone() else {
        return;
    };
    let msg_tx = tx.clone();
    tokio::spawn(async move {
        match client.walk_folder(&folder_id).await {
            Ok(walk) => {
                let base = dest_dir.join(
                    crate::format::safe_file_name(&name).unwrap_or_else(|| "download".to_string()),
                );
                // 按云端层级建本地目录(含空目录); 单个目录失败不阻断整体扫描。
                for rel in &walk.dirs {
                    let dir = join_local_path(&base, rel);
                    if let Err(e) = tokio::fs::create_dir_all(&dir).await {
                        tracing::warn!("创建本地目录失败 {}: {e}", dir.display());
                    }
                }
                // 目录条目(跳过根)与文件条目合并后按路径先序排序, 供 UI 渲染层级。
                let mut entries: Vec<(Vec<String>, FolderItem)> =
                    Vec::with_capacity(walk.dirs.len() + walk.files.len());
                for rel in &walk.dirs {
                    if rel.is_empty() {
                        continue;
                    }
                    entries.push((
                        rel.clone(),
                        FolderItem {
                            is_dir: true,
                            name: rel.last().cloned().unwrap_or_default(),
                            depth: rel.len() as u32,
                            file_id: String::new(),
                            dir: join_local_path(&base, rel),
                        },
                    ));
                }
                let mut total_bytes = 0u64;
                for (rel, f) in walk.files {
                    total_bytes = total_bytes.saturating_add(f.size.max(0) as u64);
                    let mut key = rel.clone();
                    key.push(f.name.clone());
                    entries.push((
                        key,
                        FolderItem {
                            is_dir: false,
                            name: f.name,
                            depth: rel.len() as u32 + 1,
                            file_id: f.id,
                            dir: join_local_path(&base, &rel),
                        },
                    ));
                }
                // 先序: 同一路径前缀时目录排在同名文件之前。
                entries.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.is_dir.cmp(&a.1.is_dir)));
                let items: Vec<FolderItem> = entries.into_iter().map(|(_, it)| it).collect();
                let _ = msg_tx.send(Msg::FolderScanned {
                    req_id,
                    items,
                    total_bytes,
                });
            }
            Err(e) => {
                tracing::warn!("扫描目录失败 req={req_id}: {e}");
                let _ = msg_tx.send(Msg::FolderScanFailed {
                    req_id,
                    what: format!("扫描目录失败: {e}"),
                });
            }
        }
    });
}

/// 把云端相对目录组件逐级净化后拼到本地基目录上。
fn join_local_path(base: &Path, rel: &[String]) -> PathBuf {
    let mut p = base.to_path_buf();
    for c in rel {
        p.push(crate::format::safe_file_name(c).unwrap_or_else(|| "download".to_string()));
    }
    p
}

/// 取消下载时清理未完成的 `.part` 临时文件。
fn discard_part(dest: &Path) {
    let _ = std::fs::remove_file(part_path(dest));
}

/// 任务结束后统一移除取消登记与文件名占位。
async fn cleanup_dl(
    cancel_map: &Arc<tokio::sync::Mutex<HashMap<u64, Arc<AtomicBool>>>>,
    reserved: &Arc<Mutex<HashSet<String>>>,
    req_id: u64,
    dest: &Path,
) {
    cancel_map.lock().await.remove(&req_id);
    let mut r = reserved.lock().unwrap_or_else(|e| e.into_inner());
    r.remove(&reserve_key_of(dest));
}

/// 主下载流程: 交替「解析直链 / 传输」, 两者任何一步的瞬时错误都计入次数做退避重试,
/// 每次重试前都重新解析(直链限时)。非瞬时错误立即返回。
async fn run_download(
    client: &KichiClient,
    tx: &Sender<Msg>,
    req_id: u64,
    file_id: String,
    dest: PathBuf,
    cancel: Arc<AtomicBool>,
    max_attempts: u32,
) -> Result<u64, Error> {
    // 进度转发限频, 避免高频消息刷 UI。
    let tx2 = tx.clone();
    let mut last_send = Instant::now();
    let mut on_progress = move |total: u64, done: u64| {
        let now = Instant::now();
        if done == 0 || now.duration_since(last_send) >= Duration::from_millis(150) {
            last_send = now;
            let _ = tx2.send(Msg::DlProgress {
                req_id,
                total,
                done,
            });
        }
    };

    let mut attempt: u32 = 0;
    loop {
        attempt += 1;

        // 解析(或刷新)限时直链。
        let link = match client.file_download_link(&file_id).await {
            Ok(l) => l,
            Err(e) => {
                if cancel.load(Ordering::Relaxed) || attempt >= max_attempts || !e.is_transient() {
                    return Err(e);
                }
                tokio::time::sleep(download_backoff(attempt - 1)).await;
                continue;
            }
        };

        match client
            .download_to(&link, &dest, Some(cancel.clone()), &mut on_progress)
            .await
        {
            Ok(bytes) => return Ok(bytes),
            Err(e) => {
                if cancel.load(Ordering::Relaxed) || attempt >= max_attempts || !e.is_transient() {
                    return Err(e);
                }
                tokio::time::sleep(download_backoff(attempt - 1)).await;
            }
        }
    }
}

/// 重试退避: 500ms、1s、2s…… 封顶 8s。
fn download_backoff(attempt: u32) -> Duration {
    let ms = 500u64.saturating_mul(1u64 << attempt.min(4));
    Duration::from_millis(ms.min(8000))
}

/// 同名自动加后缀: "name (n).ext"。
fn unique_name(base: &str, n: u32) -> String {
    if n == 0 {
        return base.to_string();
    }
    match base.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => {
            format!("{stem} ({n}).{ext}")
        }
        _ => format!("{base} ({n})"),
    }
}

fn reserve_key(dir: &Path, name: &str) -> String {
    format!("{}\u{0}{name}", dir.display())
}

fn reserve_key_of(dest: &Path) -> String {
    let name = dest
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let dir = dest.parent().unwrap_or(Path::new(""));
    reserve_key(dir, &name)
}

// ---------------- 本地上传调度 ----------------

/// 注册一个上传任务并 spawn 后台协程。
async fn spawn_upload(
    st: &WorkerState,
    tx: &Sender<Msg>,
    req_id: u64,
    path: PathBuf,
    parent: Option<String>,
) {
    let Some(client) = st.client.clone() else {
        return;
    };
    let cancel = Arc::new(AtomicBool::new(false));
    st.cancel.lock().await.insert(req_id, cancel.clone());
    let cancel_map = st.cancel.clone();
    let sem = st.ul_sem.clone();
    let max_attempts = st.max_attempts;
    let msg_tx = tx.clone();

    tokio::spawn(async move {
        let permit = sem.acquire().await;
        if cancel.load(Ordering::Relaxed) {
            cancel_map.lock().await.remove(&req_id);
            let _ = msg_tx.send(Msg::UlCancelled { req_id });
            drop(permit);
            return;
        }
        // 先推一条 0 进度, 让 UI 从"排队中"进入"运行/解析中"。
        let _ = msg_tx.send(Msg::UlProgress {
            req_id,
            total: 0,
            done: 0,
        });
        let outcome = run_upload(
            &client,
            &msg_tx,
            req_id,
            &path,
            parent.as_deref(),
            cancel.clone(),
            max_attempts,
        )
        .await;
        cancel_map.lock().await.remove(&req_id);
        drop(permit);

        match outcome {
            Ok(()) => {
                tracing::info!("上传完成 req={req_id}");
                let _ = msg_tx.send(Msg::UlFinished { req_id });
            }
            Err(_) if cancel.load(Ordering::Relaxed) => {
                tracing::info!("上传已取消 req={req_id}");
                let _ = msg_tx.send(Msg::UlCancelled { req_id });
            }
            Err(e) => {
                tracing::warn!("上传失败 req={req_id}: {e}");
                let _ = msg_tx.send(Msg::UlFailed {
                    req_id,
                    what: e.to_string(),
                });
            }
        }
    });
}

/// 注册一个目录上传任务并 spawn 后台协程。
async fn spawn_upload_dir(
    st: &WorkerState,
    tx: &Sender<Msg>,
    req_id: u64,
    path: PathBuf,
    parent: Option<String>,
) {
    let Some(client) = st.client.clone() else {
        return;
    };
    let cancel = Arc::new(AtomicBool::new(false));
    st.cancel.lock().await.insert(req_id, cancel.clone());
    let cancel_map = st.cancel.clone();
    let sem = st.ul_sem.clone();
    let max_attempts = st.max_attempts;
    let msg_tx = tx.clone();

    tokio::spawn(async move {
        let permit = sem.acquire().await;
        if cancel.load(Ordering::Relaxed) {
            cancel_map.lock().await.remove(&req_id);
            let _ = msg_tx.send(Msg::UlCancelled { req_id });
            drop(permit);
            return;
        }
        let _ = msg_tx.send(Msg::UlProgress {
            req_id,
            total: 0,
            done: 0,
        });
        let outcome = run_upload_dir(
            &client,
            &msg_tx,
            req_id,
            &path,
            parent.as_deref(),
            cancel.clone(),
            max_attempts,
        )
        .await;
        cancel_map.lock().await.remove(&req_id);
        drop(permit);

        match outcome {
            Ok(()) => {
                tracing::info!("目录上传完成 req={req_id}");
                let _ = msg_tx.send(Msg::UlFinished { req_id });
            }
            Err(_) if cancel.load(Ordering::Relaxed) => {
                tracing::info!("目录上传已取消 req={req_id}");
                let _ = msg_tx.send(Msg::UlCancelled { req_id });
            }
            Err(e) => {
                tracing::warn!("目录上传失败 req={req_id}: {e}");
                let _ = msg_tx.send(Msg::UlFailed {
                    req_id,
                    what: e.to_string(),
                });
            }
        }
    });
}

/// 上传主流程: 算 gcid → 创建票据(秒传则结束) → OSS 分片;
/// 分片并发且可在重试间续传, OSS 凭证失效时重建票据。
async fn run_upload(
    client: &KichiClient,
    tx: &Sender<Msg>,
    req_id: u64,
    path: &Path,
    parent: Option<&str>,
    cancel: Arc<AtomicBool>,
    max_attempts: u32,
) -> Result<(), Error> {
    let tx2 = tx.clone();
    let mut last_send = Instant::now();
    let mut on_progress = move |done: u64, total: u64| {
        let now = Instant::now();
        if done == 0 || now.duration_since(last_send) >= Duration::from_millis(150) {
            last_send = now;
            let _ = tx2.send(Msg::UlProgress {
                req_id,
                total,
                done,
            });
        }
    };
    upload_local_file(
        client,
        path,
        parent,
        &cancel,
        &mut on_progress,
        max_attempts,
    )
    .await
    .map(|_| ())
}

/// 上传单个本地文件到指定网盘目录。返回是否秒传命中。
///
/// 票据/`upload_id`/已传分片在重试间保留以实现续传; 瞬时错误退避重试,
/// OSS 凭证或 uploadId 失效(非瞬时)时丢弃票据重建后重试。
async fn upload_local_file<F>(
    client: &KichiClient,
    path: &Path,
    parent: Option<&str>,
    cancel: &Arc<AtomicBool>,
    on_progress: &mut F,
    max_attempts: u32,
) -> Result<bool, Error>
where
    F: FnMut(u64, u64) + Send,
{
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .ok_or_else(|| Error::msg("无法获取文件名"))?;
    let size = tokio::fs::metadata(path).await?.len();

    // gcid 需要完整读取文件, 放到阻塞线程池。
    let hash_path = path.to_path_buf();
    let hash = tokio::task::spawn_blocking(move || kichi_core::upload::file_gcid(&hash_path))
        .await
        .map_err(|e| Error::msg(format!("gcid 计算失败: {e}")))??;

    let mut oss: Option<OssContext> = None;
    let mut upload_id: Option<String> = None;
    let mut state = OssUploadState::default();
    let mut attempt: u32 = 0;

    loop {
        attempt += 1;

        if oss.is_none() {
            let ticket = match client.upload_create(&name, parent, size, &hash).await {
                Ok(t) => t,
                Err(e) => {
                    if cancel.load(Ordering::Relaxed)
                        || attempt >= max_attempts
                        || !e.is_transient()
                    {
                        return Err(e);
                    }
                    tokio::time::sleep(download_backoff(attempt - 1)).await;
                    continue;
                }
            };
            if ticket.completed {
                (*on_progress)(size, size);
                return Ok(true);
            }
            let Some(o) = ticket.oss else {
                return Err(Error::msg("服务端未返回上传上下文"));
            };
            match client.oss_initiate(&o).await {
                Ok(id) => {
                    upload_id = Some(id);
                    oss = Some(o);
                    state.etags.clear();
                }
                Err(e) => {
                    if cancel.load(Ordering::Relaxed)
                        || attempt >= max_attempts
                        || !e.is_transient()
                    {
                        return Err(e);
                    }
                    tokio::time::sleep(download_backoff(attempt - 1)).await;
                    continue;
                }
            }
        }

        let o = oss.as_ref().expect("oss set above");
        let id = upload_id.as_deref().expect("upload_id set above");
        match client
            .upload_oss(
                o,
                id,
                path,
                Some(cancel.clone()),
                &mut state,
                &mut *on_progress,
            )
            .await
        {
            Ok(_) => return Ok(false),
            Err(e) => {
                if cancel.load(Ordering::Relaxed) || attempt >= max_attempts {
                    return Err(e);
                }
                if !e.is_transient() {
                    // 可能是 OSS 凭证/uploadId 失效: 丢弃票据, 下轮重建。
                    oss = None;
                    upload_id = None;
                    state.etags.clear();
                }
                // 瞬时错误: 保留票据与已传分片, 下一轮续传。
                tokio::time::sleep(download_backoff(attempt - 1)).await;
            }
        }
    }
}

/// 本地目录里的一个待上传文件。
struct LocalFile {
    abs: PathBuf,
    /// 相对根目录的所在目录(根文件为空)。
    rel_dir: PathBuf,
    size: u64,
}

/// 递归收集目录内容: 返回(需创建的相对目录, 按父先于子排序; 文件; 总字节)。
fn collect_dir(root: &Path) -> Result<(Vec<PathBuf>, Vec<LocalFile>, u64), Error> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut files: Vec<LocalFile> = Vec::new();
    let mut total = 0u64;
    let mut stack: Vec<(PathBuf, PathBuf)> = vec![(PathBuf::new(), root.to_path_buf())];
    while let Some((rel, abs)) = stack.pop() {
        let rd = std::fs::read_dir(&abs)?;
        for entry in rd.flatten() {
            let Ok(ft) = entry.file_type() else { continue };
            let child_rel = rel.join(entry.file_name());
            if ft.is_symlink() {
                tracing::debug!("跳过符号链接: {}", entry.path().display());
            } else if ft.is_dir() {
                dirs.push(child_rel.clone());
                stack.push((child_rel, entry.path()));
            } else if ft.is_file() {
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                total += size;
                files.push(LocalFile {
                    abs: entry.path(),
                    rel_dir: rel.clone(),
                    size,
                });
            }
        }
    }
    Ok((dirs, files, total))
}

/// 目录递归上传: 建远端目录 + 逐个上传文件, 汇报聚合进度与文件计数。
async fn run_upload_dir(
    client: &KichiClient,
    tx: &Sender<Msg>,
    req_id: u64,
    dir: &Path,
    parent: Option<&str>,
    cancel: Arc<AtomicBool>,
    max_attempts: u32,
) -> Result<(), Error> {
    let root_name = dir
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .ok_or_else(|| Error::msg("无法获取文件夹名"))?;

    let walk = dir.to_path_buf();
    let (dirs, files, total_bytes) = tokio::task::spawn_blocking(move || collect_dir(&walk))
        .await
        .map_err(|e| Error::msg(format!("读取目录失败: {e}")))??;
    let total_files = files.len() as u32;
    tracing::info!(
        "开始上传目录 {root_name}: {total_files} 个文件, {} 字节",
        total_bytes
    );

    // 建根目录, 再按相对路径建子目录。
    let root_id = client.create_folder_id(&root_name, parent).await?;
    let mut dir_ids: HashMap<PathBuf, String> = HashMap::new();
    for rel in &dirs {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::msg("上传已取消"));
        }
        let name = rel
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let parent_rel = rel.parent().map(Path::to_path_buf).unwrap_or_default();
        let parent_id = dir_ids
            .get(&parent_rel)
            .cloned()
            .unwrap_or_else(|| root_id.clone());
        let id = client
            .create_folder_id(&name, Some(parent_id.as_str()))
            .await?;
        dir_ids.insert(rel.clone(), id);
    }

    let mut base = 0u64;
    for (idx, file) in files.into_iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::msg("上传已取消"));
        }
        let parent_id = dir_ids
            .get(&file.rel_dir)
            .cloned()
            .unwrap_or_else(|| root_id.clone());
        let cur = file
            .abs
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();

        let tx2 = tx.clone();
        let mut last_send = Instant::now();
        let base_at = base;
        let mut on_progress = move |done: u64, _total: u64| {
            let now = Instant::now();
            if done == 0 || now.duration_since(last_send) >= Duration::from_millis(150) {
                last_send = now;
                let _ = tx2.send(Msg::UlProgress {
                    req_id,
                    total: total_bytes,
                    done: base_at + done,
                });
            }
        };
        upload_local_file(
            client,
            &file.abs,
            Some(parent_id.as_str()),
            &cancel,
            &mut on_progress,
            max_attempts,
        )
        .await?;

        base += file.size;
        let _ = tx.send(Msg::UlProgress {
            req_id,
            total: total_bytes,
            done: base,
        });
        let _ = tx.send(Msg::UlFiles {
            req_id,
            done: idx as u32 + 1,
            total: total_files,
            current: cur,
        });
    }

    tracing::info!("目录上传完成: {root_name}");
    Ok(())
}

async fn refresh_quota(st: &mut WorkerState, tx: &Sender<Msg>) {
    st.last_quota_poll = Instant::now();
    let Some(client) = st.client.clone() else {
        return;
    };
    match client.quota().await {
        Ok(q) => {
            let _ = tx.send(Msg::Quota(Some(q)));
        }
        Err(e) => {
            let _ = tx.send(Msg::Quota(None));
            tracing::debug!("quota 刷新失败: {e}");
        }
    }
}

/// 会随时间自行变化的离线任务状态(其余状态只在用户操作时改变)。
fn is_active_phase(phase: &str) -> bool {
    matches!(phase, "PHASE_TYPE_PENDING" | "PHASE_TYPE_RUNNING")
}

async fn refresh_tasks(st: &mut WorkerState, tx: &Sender<Msg>) {
    st.last_tasks_poll = Instant::now();
    let Some(client) = st.client.clone() else {
        return;
    };
    let mut buckets: BTreeMap<String, Vec<serde_json::Value>> = BTreeMap::new();
    let mut nexts: BTreeMap<String, Option<String>> = BTreeMap::new();
    let mut active = false;
    let mut active_unknown = false;
    for phase in OFFLINE_PHASES {
        // 保持用户已加载的分页深度(至少 1 页)。
        let want = st.tasks_pages.get(phase).copied().unwrap_or(1).max(1);
        let mut all: Vec<serde_json::Value> = Vec::new();
        let mut token: Option<String> = None;
        let mut fetched = 0usize;
        let mut ok = true;
        for _ in 0..want {
            match client
                .offline_list_phase(phase, TASK_PAGE_SIZE, token.as_deref())
                .await
            {
                Ok(page) => {
                    all.extend(page.tasks);
                    token = page.next_page_token;
                    fetched += 1;
                }
                Err(e) => {
                    tracing::debug!("离线任务[{phase}] 刷新失败: {e}");
                    ok = false;
                    break;
                }
            }
            if token.is_none() {
                break;
            }
        }
        if !ok {
            // 单个分桶失败时保留旧数据(不插入), 连同其旧游标。
            // 进行中分桶失败时状态未知, 维持上一轮的活动判定(避免误降频)。
            if is_active_phase(phase) {
                active_unknown = true;
            }
            continue;
        }
        if is_active_phase(phase) && !all.is_empty() {
            active = true;
        }
        if all.is_empty() {
            st.tasks_pages.remove(phase);
            st.tasks_next.remove(phase);
        } else {
            st.tasks_pages.insert(phase.to_string(), fetched.max(1));
            st.tasks_next.insert(phase.to_string(), token.clone());
        }
        buckets.insert(phase.to_string(), all);
        nexts.insert(phase.to_string(), token);
    }
    st.tasks_active = active || (active_unknown && st.tasks_active);
    let _ = tx.send(Msg::TasksAll {
        buckets,
        next_tokens: nexts,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_local_path_sanitizes_components() {
        let base = Path::new("/tmp/dl/A");
        // 空相对路径 = 目录本身。
        assert_eq!(join_local_path(base, &[]), PathBuf::from("/tmp/dl/A"));
        // 逐级拼接。
        assert_eq!(
            join_local_path(base, &["B".into(), "C".into()]),
            PathBuf::from("/tmp/dl/A/B/C")
        );
    }

    #[test]
    fn join_local_path_strips_separators_and_rejects_traversal() {
        let base = Path::new("/tmp/dl");
        // 组件内的路径分隔被收敛为 basename, 路径穿越被拒。
        assert_eq!(
            join_local_path(base, &["../evil".into(), ".".into()]),
            PathBuf::from("/tmp/dl/evil/download")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn gate_limit_changes_take_effect_immediately() {
        let gate = Arc::new(Gate::new(1));
        let held = gate.acquire().await; // 占满唯一槽位

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let g2 = gate.clone();
        let waiter = tokio::spawn(async move {
            let _g = g2.acquire().await;
            let _ = tx.send(());
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(rx.try_recv().is_err(), "满载时新任务应排队");

        // 扩容立刻放行排队任务。
        gate.set_limit(2);
        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("扩容后未放行")
            .expect("排队任务 panic");

        // 缩容不打断已在传的任务(held 仍持有槽位), 但超额后的新 acquire 需排队。
        gate.set_limit(1);
        let g3 = gate.clone();
        let blocked = tokio::spawn(async move {
            let _g = g3.acquire().await;
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!blocked.is_finished(), "缩容后超额时新任务应排队");

        // 槽位释放后排队任务自动获准。
        drop(held);
        tokio::time::timeout(Duration::from_secs(1), blocked)
            .await
            .expect("槽位释放后未放行排队任务")
            .expect("排队任务 panic");
    }
}
