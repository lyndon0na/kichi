mod auth;
mod cache;
mod download;
mod gate;
mod preview;
mod shares;
mod tasks;
mod thumbs;
mod trash;
mod upload;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kichi_core::download::part_path;
use kichi_core::KichiClient;
use tokio::sync::Semaphore;

use crate::msg::{Cmd, Msg};

use self::cache::{sweep_caches, CacheCtl};
use self::gate::Gate;
use self::tasks::{TASKS_POLL_ACTIVE, TASKS_POLL_IDLE};
use self::thumbs::THUMB_CONCURRENCY;
// UI 侧「预览大文件」确认要查询缓存命中, 对外仍以 worker::preview_cached 暴露。
pub(crate) use self::preview::preview_cached;

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

/// 后台空闲轮询基节拍(仅作 recv 超时上限, 命令到达会立即唤醒)。
const POLL_TICK: Duration = Duration::from_millis(800);
/// 配额轮询节拍: 变化慢, 且登录与删除等操作后已显式刷新。
const QUOTA_POLL: Duration = Duration::from_secs(60);

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
    /// 缩略图下载并发闸。
    thumb_sem: Arc<Semaphore>,
    /// 缩略图请求的目录代数: 目录切换 / 刷新 / 新搜索时自增,
    /// 使在跑的旧任务在检查点自行放弃, 不再下载已经离开视野的图。
    thumb_gen: Arc<AtomicU64>,
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
        thumb_sem: Arc::new(Semaphore::new(THUMB_CONCURRENCY)),
        thumb_gen: Arc::new(AtomicU64::new(0)),
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
                        tasks::refresh_tasks(&mut st, &tx).await;
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
            auth::do_login(st, tx, username, password).await;
        }
        Cmd::AutoLogin { username } => auth::auto_login(st, tx, username).await,
        Cmd::RememberPassword { username, password } => {
            auth::remember_password(tx, username, password).await
        }
        Cmd::ForgetPassword { username } => auth::forget_password(username).await,
        Cmd::Resume {
            device_id,
            access_token,
            refresh_token,
            user_id,
            username,
        } => {
            auth::resume_session(
                st,
                tx,
                device_id,
                access_token,
                refresh_token,
                user_id,
                username,
            )
            .await
        }
        Cmd::Logout => auth::logout(st, tx).await,
        Cmd::ListFiles {
            parent,
            token,
            append,
            req_id,
        } => {
            let Some(client) = &st.client else { return };
            if !append {
                // 切换目录 / 刷新: 让在跑的缩略图任务作废(分页加载不算)。
                st.thumb_gen.fetch_add(1, Ordering::Relaxed);
            }
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
            if !append {
                // 新一次搜索 = 换了一批显示内容, 旧的缩略图任务不再有意义。
                st.thumb_gen.fetch_add(1, Ordering::Relaxed);
            }
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
        Cmd::Trash { ids } => trash::trash_files(st, tx, ids).await,
        Cmd::ListTrash {
            token,
            append,
            req_id,
        } => trash::list_trash(st, tx, token, append, req_id).await,
        Cmd::Untrash { ids } => trash::untrash_files(st, tx, ids).await,
        Cmd::DeleteTrash { ids } => trash::delete_trash_files(st, tx, ids).await,
        Cmd::EmptyTrash => trash::empty_trash(st, tx).await,
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
            tasks::offline_create(st, tx, url, name, parent).await
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
        Cmd::OfflineRetry { task_id } => tasks::offline_retry(st, tx, task_id).await,
        Cmd::OfflineDelete {
            task_ids,
            delete_files,
        } => tasks::offline_delete(st, tx, task_ids, delete_files).await,
        Cmd::RefreshTasks => tasks::refresh_tasks(st, tx).await,
        Cmd::LoadMoreTasks { phase } => tasks::load_more_tasks(st, tx, phase).await,
        Cmd::RefreshQuota => refresh_quota(st, tx).await,
        Cmd::StartDownload {
            req_id,
            file_id,
            name,
            dest_dir,
        } => {
            download::spawn_download(st, tx, req_id, file_id, name, dest_dir).await;
        }
        Cmd::StartDownloadFolder {
            req_id,
            folder_id,
            name,
            dest_dir,
        } => {
            download::spawn_folder_download(st, tx, req_id, folder_id, name, dest_dir).await;
        }
        Cmd::CancelDownload { req_id } => cancel_task(st, req_id).await,
        Cmd::StartUpload {
            req_id,
            path,
            parent,
        } => {
            upload::spawn_upload(st, tx, req_id, path, parent).await;
        }
        Cmd::StartUploadDir {
            req_id,
            path,
            parent,
        } => {
            upload::spawn_upload_dir(st, tx, req_id, path, parent).await;
        }
        Cmd::CancelUpload { req_id } => cancel_task(st, req_id).await,
        Cmd::Preview {
            req_id,
            file_id,
            name,
            media,
            subtitles,
        } => preview::spawn_preview(st, tx, req_id, file_id, name, media, subtitles).await,
        Cmd::CancelPreview { req_id } => cancel_task(st, req_id).await,
        Cmd::PreviewQualities { file_id, subtitles } => {
            preview::spawn_preview_qualities(st, tx, file_id, subtitles)
        }
        Cmd::CreateShare {
            file_ids,
            expiration_days,
            need_password,
            label,
        } => shares::create_share(st, tx, file_ids, expiration_days, need_password, label).await,
        Cmd::ListShares {
            token,
            append,
            req_id,
        } => shares::list_shares(st, tx, token, append, req_id).await,
        Cmd::DeleteShares { ids } => shares::delete_shares(st, tx, ids).await,
        Cmd::ResolveShare {
            share_id,
            pass_code,
        } => shares::resolve_share(st, tx, share_id, pass_code).await,
        Cmd::LoadMoreShareFiles {
            share_id,
            pass_code_token,
            page_token,
        } => shares::load_more_share_files(st, tx, share_id, pass_code_token, page_token).await,
        Cmd::SaveShare {
            share_id,
            pass_code_token,
            file_ids,
            dest,
        } => shares::save_share(st, tx, share_id, pass_code_token, file_ids, dest).await,
        Cmd::RetryMoveShare { dest } => shares::retry_move_share(st, tx, dest).await,
        Cmd::LoadThumbnail {
            file_id,
            url,
            max_edge,
        } => thumbs::spawn_thumbnail(st, tx, file_id, url, max_edge),
        Cmd::SetTransferLimits {
            dl_concurrency,
            ul_concurrency,
            part_concurrency,
            max_attempts,
        } => set_transfer_limits(
            st,
            dl_concurrency,
            ul_concurrency,
            part_concurrency,
            max_attempts,
        ),
        Cmd::MaintainCache { purge } => cache::maintain_cache(st.cache.clone(), tx.clone(), purge),
    }
}

/// 取消下载时清理未完成的 `.part` 临时文件。
fn discard_part(dest: &Path) {
    let _ = std::fs::remove_file(part_path(dest));
}

/// 重试退避: 500ms、1s、2s…… 封顶 8s。
fn download_backoff(attempt: u32) -> Duration {
    let ms = 500u64.saturating_mul(1u64 << attempt.min(4));
    Duration::from_millis(ms.min(8000))
}

/// 置位取消标志: 下载 / 上传 / 预览共用同一取消注册表(按 req_id 索引)。
async fn cancel_task(st: &WorkerState, req_id: u64) {
    let map = st.cancel.lock().await;
    if let Some(flag) = map.get(&req_id) {
        flag.store(true, Ordering::Relaxed);
    }
}

/// 并发即时生效: 扩容立刻放行排队任务, 缩容只影响后续 acquire, 不打断在传任务。
fn set_transfer_limits(
    st: &mut WorkerState,
    dl_concurrency: usize,
    ul_concurrency: usize,
    part_concurrency: usize,
    max_attempts: usize,
) {
    st.sem.set_limit(dl_concurrency);
    st.ul_sem.set_limit(ul_concurrency);
    st.max_attempts = max_attempts as u32;
    st.part_concurrency = part_concurrency;
    if let Some(client) = &st.client {
        client.set_part_concurrency(part_concurrency);
    }
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
