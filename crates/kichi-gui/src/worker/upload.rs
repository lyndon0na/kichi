use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::{Duration, Instant};

use kichi_core::upload::{OssContext, OssUploadState};
use kichi_core::{Error, KichiClient};

use crate::msg::Msg;

use super::{download_backoff, WorkerState};

/// 注册一个上传任务并 spawn 后台协程。
pub(super) async fn spawn_upload(
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
pub(super) async fn spawn_upload_dir(
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
                &mut |_| {},
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
