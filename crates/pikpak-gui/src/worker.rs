use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pikpak_core::consts::OFFLINE_PHASES;
use pikpak_core::download::part_path;
use pikpak_core::upload::{OssContext, OssUploadState};
use pikpak_core::{session, Error, PikPakClient};
use tokio::sync::Semaphore;

use crate::credentials;
use crate::msg::{Cmd, Msg, QualityOption};

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
/// 本地上传的并发上限与单任务最大尝试次数。
const UL_CONCURRENCY: usize = 2;
const UL_MAX_ATTEMPTS: u32 = 5;

struct WorkerState {
    client: Option<Arc<PikPakClient>>,
    /// 下载/上传取消开关, 按 req_id 索引; 任务结束后自行移除。
    cancel: Arc<tokio::sync::Mutex<HashMap<u64, Arc<AtomicBool>>>>,
    /// 下载并发信号量, 超出上限的任务阻塞在 acquire 上排队。
    sem: Arc<Semaphore>,
    /// 上传并发信号量。
    ul_sem: Arc<Semaphore>,
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
        ul_sem: Arc::new(Semaphore::new(UL_CONCURRENCY)),
        reserved: Arc::new(Mutex::new(HashSet::new())),
    };
    let mut tick = 0u64;

    tracing::info!("后台 worker 已启动");
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
            let mut client = PikPakClient::new(device_id);
            install_saver(&mut client);
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
            let Some(client) = st.client.clone() else { return };
            let tx = tx.clone();
            tokio::spawn(async move {
                if media {
                    preview_stream(&client, &tx, req_id, file_id, name, subtitles).await;
                } else {
                    preview_download(&client, &tx, req_id, file_id, name).await;
                }
            });
        }
        Cmd::PreviewQualities {
            file_id,
            subtitles,
        } => {
            let Some(client) = st.client.clone() else { return };
            let tx = tx.clone();
            tokio::spawn(async move {
                preview_qualities(&client, &tx, file_id, subtitles).await;
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
    }
}

/// 账号密码登录的公共实现(手动登录与密钥环自动登录共用)。
async fn do_login(st: &mut WorkerState, tx: &Sender<Msg>, username: String, password: String) {
    let device_id = pikpak_core::captcha::generate_device_id();
    let mut client = PikPakClient::new(device_id.clone());
    install_saver(&mut client);
    tracing::info!("开始登录: {username}");
    match client.login(&username, &password).await {
        Ok(sess) => {
            if let Err(e) = session::save_session(&sess) {
                let _ = tx.send(Msg::Error { what: e.to_string() });
            }
            st.client = Some(Arc::new(client));
            tracing::info!("登录成功: {username}");
            let _ = tx.send(Msg::LoginOk { username });
            refresh_quota(st, tx).await;
            refresh_tasks(st, tx).await;
        }
        Err(e) => {
            tracing::warn!("登录失败: {e}");
            let _ = tx.send(Msg::LoginFailed {
                what: format!("登录失败: {e}"),
            });
        }
    }
}

fn install_saver(client: &mut PikPakClient) {
    let saver: pikpak_core::client::TokenSaver = Arc::new(|sess| {
        let _ = session::save_session(sess);
    });
    client.set_token_saver(saver);
}

// ---------------- 文件预览 ----------------

/// 预览缓存目录: `~/.cache/pikpak-linux/preview`(取不到时退回临时目录)。
fn preview_root() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("pikpak-linux")
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
    client: &PikPakClient,
    tx: &Sender<Msg>,
    req_id: u64,
    file_id: String,
    name: String,
    subtitles: Vec<(String, String)>,
) {
    let subs = prepare_subtitles(client, &subtitles).await;
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
    client: &PikPakClient,
    tx: &Sender<Msg>,
    file_id: String,
    subtitles: Vec<(String, String)>,
) {
    let subs = prepare_subtitles(client, &subtitles).await;
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
    client: &PikPakClient,
    subtitles: &[(String, String)],
) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for (id, name) in subtitles {
        let dest = preview_cache_path(id, name);
        if dest.exists() {
            paths.push(dest);
            continue;
        }
        let Ok(link) = client.file_download_link(id).await else {
            continue;
        };
        if client
            .download_to(&link, &dest, None, |_, _| {})
            .await
            .is_ok()
        {
            paths.push(dest);
        }
    }
    paths
}

/// 非媒体预览: 下载到本地缓存(命中缓存则跳过), 再交给系统查看器打开。
async fn preview_download(
    client: &PikPakClient,
    tx: &Sender<Msg>,
    req_id: u64,
    file_id: String,
    name: String,
) {
    let dest = preview_cache_path(&file_id, &name);
    if dest.exists() {
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
    match client
        .download_to(&link, &dest, None, |_, _| {})
        .await
    {
        Ok(_) => {
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
    let reserved = st.reserved.clone();
    let msg_tx = tx.clone();

    tokio::spawn(async move {
        // 抢占并发槽位; 若等待期间被取消则直接退出(释放占位)。
        let permit = sem.acquire().await.ok();
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
    let msg_tx = tx.clone();

    tokio::spawn(async move {
        let permit = sem.acquire().await.ok();
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
        let outcome =
            run_upload(&client, &msg_tx, req_id, &path, parent.as_deref(), cancel.clone()).await;
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
    let msg_tx = tx.clone();

    tokio::spawn(async move {
        let permit = sem.acquire().await.ok();
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
        let outcome =
            run_upload_dir(&client, &msg_tx, req_id, &path, parent.as_deref(), cancel.clone())
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
    client: &PikPakClient,
    tx: &Sender<Msg>,
    req_id: u64,
    path: &Path,
    parent: Option<&str>,
    cancel: Arc<AtomicBool>,
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
    upload_local_file(client, path, parent, &cancel, &mut on_progress)
        .await
        .map(|_| ())
}

/// 上传单个本地文件到指定网盘目录。返回是否秒传命中。
///
/// 票据/`upload_id`/已传分片在重试间保留以实现续传; 瞬时错误退避重试,
/// OSS 凭证或 uploadId 失效(非瞬时)时丢弃票据重建后重试。
async fn upload_local_file<F>(
    client: &PikPakClient,
    path: &Path,
    parent: Option<&str>,
    cancel: &Arc<AtomicBool>,
    on_progress: &mut F,
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
    let hash = tokio::task::spawn_blocking(move || pikpak_core::upload::file_gcid(&hash_path))
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
                        || attempt >= UL_MAX_ATTEMPTS
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
                        || attempt >= UL_MAX_ATTEMPTS
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
            .upload_oss(o, id, path, Some(cancel.clone()), &mut state, &mut *on_progress)
            .await
        {
            Ok(_) => return Ok(false),
            Err(e) => {
                if cancel.load(Ordering::Relaxed) || attempt >= UL_MAX_ATTEMPTS {
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
    client: &PikPakClient,
    tx: &Sender<Msg>,
    req_id: u64,
    dir: &Path,
    parent: Option<&str>,
    cancel: Arc<AtomicBool>,
) -> Result<(), Error> {
    let root_name = dir
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .ok_or_else(|| Error::msg("无法获取文件夹名"))?;

    let walk = dir.to_path_buf();
    let (dirs, files, total_bytes) =
        tokio::task::spawn_blocking(move || collect_dir(&walk))
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
        let parent_id = dir_ids.get(&parent_rel).cloned().unwrap_or_else(|| root_id.clone());
        let id = client.create_folder_id(&name, Some(parent_id.as_str())).await?;
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
        upload_local_file(client, &file.abs, Some(parent_id.as_str()), &cancel, &mut on_progress)
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
    for phase in OFFLINE_PHASES {
        match client.offline_list_phase(phase, 100, None).await {
            Ok(tasks) => {
                buckets.insert(phase.to_string(), tasks.tasks);
            }
            Err(e) => {
                // 单个分桶失败时保留旧数据, 不阻断其它分桶的刷新。
                tracing::debug!("离线任务[{phase}] 刷新失败: {e}");
            }
        }
    }
    let _ = tx.send(Msg::TasksAll { buckets });
}
