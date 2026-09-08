use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pikpak_core::consts::OFFLINE_PHASES;
use pikpak_core::{session, Error, PikPakClient};
use tokio::sync::Semaphore;

use crate::msg::{Cmd, Msg};

pub struct Worker {
    pub tx: Sender<Cmd>,
    pub rx: Receiver<Msg>,
}

pub fn spawn() -> Worker {
    let (cmd_tx, cmd_rx) = mpsc::channel::<Cmd>();
    let (msg_tx, msg_rx) = mpsc::channel::<Msg>();
    std::thread::Builder::new()
        .name("pikpak-worker".into())
        .spawn(move || run(cmd_rx, msg_tx))
        .expect("failed to spawn worker");
    Worker {
        tx: cmd_tx,
        rx: msg_rx,
    }
}

/// 本地下载的并发上限与单任务最大下载尝试次数。
const DL_CONCURRENCY: usize = 3;
const DL_MAX_ATTEMPTS: u32 = 5;

struct WorkerState {
    client: Option<Arc<PikPakClient>>,
    /// 下载取消开关, 按 req_id 索引; 任务结束后自行移除。
    cancel: Arc<tokio::sync::Mutex<HashMap<u64, Arc<AtomicBool>>>>,
    /// 并发信号量, 超出上限的任务阻塞在 acquire 上排队。
    sem: Arc<Semaphore>,
    /// 已占用的目标文件名集合(键: "目录\0文件名"), 用于同名去重。
    reserved: Arc<Mutex<HashSet<String>>>,
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
    let mut st = WorkerState {
        client: None,
        cancel: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        sem: Arc::new(Semaphore::new(DL_CONCURRENCY)),
        reserved: Arc::new(Mutex::new(HashSet::new())),
    };
    let mut tick = 0u64;

    loop {
        let msg = rx.recv_timeout(Duration::from_millis(800));

        match msg {
            Ok(cmd) => {
                handle(&mut st, &tx, cmd).await;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // 周期性自动刷新。
                if st.client.is_some() {
                    tick += 1;
                    if tick.is_multiple_of(3) {
                        refresh_quota(&st, &tx).await;
                    }
                    if tick.is_multiple_of(6) {
                        refresh_tasks(&st, &tx).await;
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
            let device_id = pikpak_core::captcha::generate_device_id();
            let mut client = PikPakClient::new(device_id.clone());
            install_saver(&mut client);
            match client.login(&username, &password).await {
                Ok(sess) => {
                    if let Err(e) = session::save_session(&sess) {
                        let _ = tx.send(Msg::Error {
                            what: e.to_string(),
                        });
                    }
                    st.client = Some(Arc::new(client));
                    let _ = tx.send(Msg::LoginOk { username });
                    refresh_quota(st, tx).await;
                    refresh_tasks(st, tx).await;
                }
                Err(e) => {
                    let _ = tx.send(Msg::Error {
                        what: format!("登录失败: {e}"),
                    });
                }
            }
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
            let mut client = PikPakClient::new(device_id);
            install_saver(&mut client);
            client.set_session(&sess).await;
            let client = Arc::new(client);
            match client.quota().await {
                Ok(_) => {
                    st.client = Some(client);
                    let _ = tx.send(Msg::LoginOk { username });
                    refresh_quota(st, tx).await;
                    refresh_tasks(st, tx).await;
                }
                // 只有服务端明确判定凭据失效(API 错误)才强制重新登录;
                // 其余(网络/解析等)先保留本地会话, 由后台周期刷新自动重试。
                Err(e @ Error::Api { .. }) => {
                    let _ = tx.send(Msg::SessionInvalid {
                        reason: format!("登录已过期: {e}"),
                    });
                }
                Err(e) => {
                    st.client = Some(client);
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
                let mut r = st.reserved.lock().unwrap();
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
                    let _ = tx.send(Msg::Error {
                        what: format!("加载文件列表失败: {e}"),
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
        Cmd::RefreshQuota => refresh_quota(st, tx).await,
        Cmd::StartDownload {
            req_id,
            file_id,
            name,
            dest_dir,
        } => {
            spawn_download(st, tx, req_id, file_id, name, dest_dir).await;
        }
        Cmd::CancelDownload { req_id } => {
            let map = st.cancel.lock().await;
            if let Some(flag) = map.get(&req_id) {
                flag.store(true, Ordering::Relaxed);
            }
        }
    }
}

fn install_saver(client: &mut PikPakClient) {
    let saver: pikpak_core::client::TokenSaver = Arc::new(|sess| {
        let _ = session::save_session(sess);
    });
    client.set_token_saver(saver);
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
        let mut reserved = st.reserved.lock().unwrap();
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
    let reserved = st.reserved.clone();
    let msg_tx = tx.clone();

    tokio::spawn(async move {
        // 抢占并发槽位; 若等待期间被取消则直接退出(释放占位)。
        let permit = sem.acquire().await.ok();
        if cancel.load(Ordering::Relaxed) {
            cleanup_dl(&cancel_map, &reserved, req_id, &dest).await;
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
        )
        .await;
        cleanup_dl(&cancel_map, &reserved, req_id, &dest).await;
        drop(permit);

        match outcome {
            Ok(bytes) => {
                let _ = msg_tx.send(Msg::DlFinished { req_id, bytes });
            }
            Err(e) => {
                if cancel.load(Ordering::Relaxed) {
                    let _ = msg_tx.send(Msg::DlCancelled { req_id });
                } else {
                    let _ = msg_tx.send(Msg::DlFailed {
                        req_id,
                        what: e.to_string(),
                    });
                }
            }
        }
    });
}

/// 任务结束后统一移除取消登记与文件名占位。
async fn cleanup_dl(
    cancel_map: &Arc<tokio::sync::Mutex<HashMap<u64, Arc<AtomicBool>>>>,
    reserved: &Arc<Mutex<HashSet<String>>>,
    req_id: u64,
    dest: &Path,
) {
    cancel_map.lock().await.remove(&req_id);
    let mut r = reserved.lock().unwrap();
    r.remove(&reserve_key_of(dest));
}

/// 主下载流程: 交替「解析直链 / 传输」, 两者任何一步的瞬时错误都计入次数做退避重试,
/// 每次重试前都重新解析(直链限时)。非瞬时错误立即返回。
async fn run_download(
    client: &PikPakClient,
    tx: &Sender<Msg>,
    req_id: u64,
    file_id: String,
    dest: PathBuf,
    cancel: Arc<AtomicBool>,
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
                if cancel.load(Ordering::Relaxed) || attempt >= DL_MAX_ATTEMPTS || !e.is_transient()
                {
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
                if cancel.load(Ordering::Relaxed) || attempt >= DL_MAX_ATTEMPTS || !e.is_transient()
                {
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

async fn refresh_quota(st: &WorkerState, tx: &Sender<Msg>) {
    let Some(client) = &st.client else { return };
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

async fn refresh_tasks(st: &WorkerState, tx: &Sender<Msg>) {
    let Some(client) = &st.client else { return };
    let mut buckets: BTreeMap<String, Vec<serde_json::Value>> = BTreeMap::new();
    let mut ok = true;
    for phase in OFFLINE_PHASES {
        match client.offline_list_phase(phase, 100, None).await {
            Ok(tasks) => {
                buckets.insert(phase.to_string(), tasks.tasks);
            }
            Err(e) => {
                ok = false;
                tracing::debug!("离线任务[{phase}] 刷新失败: {e}");
                break;
            }
        }
    }
    if ok {
        let _ = tx.send(Msg::TasksAll { buckets });
    } else {
        tracing::debug!("离线任务批量刷新失败, 已跳过本轮");
    }
}
