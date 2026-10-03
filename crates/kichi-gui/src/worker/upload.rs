use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::{Duration, Instant};

use kichi_core::upload::{upload_chunk_size, OssContext, OssUploadState};
use kichi_core::{Error, KichiClient};

use crate::msg::Msg;
use crate::settings;

use super::{download_backoff, WorkerState};

/// 注册一个上传任务并 spawn 后台协程。
pub(super) async fn spawn_upload(
    st: &WorkerState,
    tx: &Sender<Msg>,
    req_id: u64,
    path: PathBuf,
    parent: Option<String>,
    dest_stack: Vec<(Option<String>, String)>,
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
            discard_resume(Some(&client), &path, parent.as_deref());
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
            Some(&dest_stack),
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
                // 取消即放弃续传: 删除落盘记录, 云端占位条目走后台清理。
                discard_resume(Some(&client), &path, parent.as_deref());
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
#[allow(clippy::too_many_arguments)]
async fn run_upload(
    client: &Arc<KichiClient>,
    tx: &Sender<Msg>,
    req_id: u64,
    path: &Path,
    parent: Option<&str>,
    dest_stack: Option<&[(Option<String>, String)]>,
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
        tx,
        req_id,
        path,
        parent,
        dest_stack,
        &cancel,
        &mut on_progress,
        max_attempts,
    )
    .await
    .map(|_| ())
}

/// 上传单个本地文件到指定网盘目录。返回是否秒传命中。
///
/// `dest_stack` 为 Some 时启用跨重启续传(落盘凭证与已传分片断点); 目录递归上传
/// 的子文件传 None(不续传)。续传命中时跳过 gcid 计算与取票; 记录失效(过期 /
/// 本地文件已变)时丢弃记录与旧占位条目, 退回全量上传。
#[allow(clippy::too_many_arguments)]
async fn upload_local_file<F>(
    client: &Arc<KichiClient>,
    tx: &Sender<Msg>,
    req_id: u64,
    path: &Path,
    parent: Option<&str>,
    dest_stack: Option<&[(Option<String>, String)]>,
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
    let meta = tokio::fs::metadata(path).await?;
    let size = meta.len();
    let mtime = settings::mtime_ms(&meta);
    let persist = dest_stack.is_some();

    let mut oss: Option<OssContext> = None;
    let mut upload_id: Option<String> = None;
    let mut state = OssUploadState::default();
    // 仅在需要重新取票时计算(续传命中时可省去大文件重哈希)。
    let mut hash: Option<String> = None;
    let mut attempt: u32 = 0;

    // 落盘续传: 凭证仍在有效期内且本地文件指纹一致时, 免取票直接续传。
    if persist {
        if let Some(rec) = settings::find_upload_resume(path, parent) {
            if rec.usable(crate::format::now_unix() as i64, size, mtime) {
                oss = Some(OssContext {
                    endpoint: rec.oss.endpoint,
                    access_key_id: rec.oss.access_key_id,
                    access_key_secret: rec.oss.access_key_secret,
                    security_token: rec.oss.security_token,
                    bucket: rec.oss.bucket,
                    key: rec.oss.key,
                });
                upload_id = Some(rec.upload_id);
                state.etags = rec.etags.into_iter().collect();
                let skipped = state.uploaded_bytes(upload_chunk_size(size), size);
                tracing::info!(
                    "上传续传: 复用 {} 个已传分片(跳过 {skipped} 字节)",
                    state.etags.len()
                );
                let _ = tx.send(Msg::UlResumed {
                    req_id,
                    resumed: true,
                    skipped,
                    total: size,
                    note: None,
                });
            } else {
                // 凭证过期 / 本地文件已变: 丢弃记录与旧占位条目, 全量重来。
                discard_resume(Some(client), path, parent);
                tracing::info!("上传续传记录已失效, 改为全量上传");
                let _ = tx.send(Msg::UlResumed {
                    req_id,
                    resumed: false,
                    skipped: 0,
                    total: size,
                    note: Some("上传凭证已过期, 已重新开始上传".into()),
                });
            }
        }
    }

    // 已传分片的落盘(节流 1s; 完成 / 取消时记录整体删除, 无需补记最后一次)。
    let mut last_persist = Instant::now();
    let mut on_etags = |st: &OssUploadState| {
        if !persist || last_persist.elapsed() < Duration::from_millis(1000) {
            return;
        }
        last_persist = Instant::now();
        let etags: BTreeMap<u64, String> = st.etags.iter().map(|(p, e)| (*p, e.clone())).collect();
        settings::update_upload_resume_etags(path, parent, &etags);
    };

    loop {
        attempt += 1;

        if oss.is_none() {
            let hash_value = match &hash {
                Some(h) => h.clone(),
                None => {
                    // gcid 需要完整读取文件, 放到阻塞线程池。
                    let hash_path = path.to_path_buf();
                    let h = tokio::task::spawn_blocking(move || {
                        kichi_core::upload::file_gcid(&hash_path)
                    })
                    .await
                    .map_err(|e| Error::msg(format!("gcid 计算失败: {e}")))??;
                    hash = Some(h.clone());
                    h
                }
            };
            let ticket = match client.upload_create(&name, parent, size, &hash_value).await {
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
            // 占位条目 id 与凭证到期时间只用于落盘记录(续传命中时不重建记录)。
            let file_id = ticket.file_id.clone();
            let expiration_unix = ticket.expiration_unix;
            match client.oss_initiate(&o).await {
                Ok(id) => {
                    state.etags.clear();
                    if persist {
                        settings::upsert_upload_resume(settings::UploadResumeRecord {
                            local_path: path.to_path_buf(),
                            name: name.clone(),
                            parent: parent.map(str::to_string),
                            dest_stack: dest_stack.unwrap_or_default().to_vec(),
                            size,
                            mtime_ms: mtime,
                            file_id,
                            upload_id: id.clone(),
                            oss: settings::UploadResumeOss {
                                endpoint: o.endpoint.clone(),
                                access_key_id: o.access_key_id.clone(),
                                access_key_secret: o.access_key_secret.clone(),
                                security_token: o.security_token.clone(),
                                bucket: o.bucket.clone(),
                                key: o.key.clone(),
                            },
                            expiration_unix,
                            etags: BTreeMap::new(),
                            at: crate::format::now_unix(),
                        });
                    }
                    upload_id = Some(id);
                    oss = Some(o);
                }
                Err(e) => {
                    // 重试会重新取票, 本张票据的占位条目不再使用, 尽快清掉。
                    if let Some(fid) = file_id {
                        spawn_placeholder_cleanup(
                            client.clone(),
                            parent.map(str::to_string),
                            Some(fid),
                        );
                    }
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
                &mut on_etags,
            )
            .await
        {
            Ok(_) => {
                if persist {
                    settings::take_upload_resume(path, parent);
                }
                return Ok(false);
            }
            Err(e) => {
                if cancel.load(Ordering::Relaxed) || attempt >= max_attempts {
                    return Err(e);
                }
                if !e.is_transient() {
                    // 可能是 OSS 凭证/uploadId 失效: 丢弃票据与落盘记录, 下轮重建。
                    oss = None;
                    upload_id = None;
                    state.etags.clear();
                    discard_resume(Some(client), path, parent);
                }
                // 瞬时错误: 保留票据与已传分片(含落盘记录), 下一轮续传。
                tokio::time::sleep(download_backoff(attempt - 1)).await;
            }
        }
    }
}

/// 云端占位条目清理: 列表页大小与最大翻页数(防异常目录下无限翻页)。
const PENDING_PAGE_SIZE: usize = 100;
const PENDING_MAX_PAGES: usize = 20;

/// 丢弃上传续传记录(取消 / 显式移除 / 票据作废时)。记录删除是即时的;
/// 云端 PENDING 占位条目清理走后台, 失败只记日志。
pub(super) fn discard_resume(client: Option<&Arc<KichiClient>>, path: &Path, parent: Option<&str>) {
    let Some(rec) = settings::take_upload_resume(path, parent) else {
        return;
    };
    if let Some(client) = client {
        spawn_placeholder_cleanup(client.clone(), rec.parent, rec.file_id);
    }
}

/// 后台尽力清理一个 PENDING 占位条目。
fn spawn_placeholder_cleanup(
    client: Arc<KichiClient>,
    parent: Option<String>,
    file_id: Option<String>,
) {
    if file_id.is_none() {
        return;
    }
    tokio::spawn(async move {
        cleanup_placeholder(&client, parent.as_deref(), file_id.as_deref()).await;
    });
}

/// 清理云端占位条目: 必须确认它仍是 PENDING 才删 —— `oss_complete` 成功但响应
/// 丢失的窗口里条目可能已转为正式文件, 误删会丢用户数据。
async fn cleanup_placeholder(client: &KichiClient, parent: Option<&str>, file_id: Option<&str>) {
    let Some(fid) = file_id else {
        return;
    };
    let mut token: Option<String> = None;
    for _ in 0..PENDING_MAX_PAGES {
        let list = match client
            .pending_placeholders(parent, PENDING_PAGE_SIZE, token.as_deref())
            .await
        {
            Ok(l) => l,
            Err(e) => {
                tracing::debug!("占位条目清理: 列表请求失败, 忽略: {e}");
                return;
            }
        };
        if list.files.iter().any(|f| f.id == fid) {
            let ids = [fid.to_string()];
            let _ = client.batch_trash(&ids).await;
            let _ = client.batch_delete(&ids).await;
            tracing::info!("已清理上传占位条目");
            return;
        }
        match list.next_page_token {
            Some(t) => token = Some(t),
            None => {
                tracing::debug!("占位条目已不是 PENDING(可能已完成), 跳过清理");
                return;
            }
        }
    }
    tracing::debug!("占位条目未在限定页数内找到, 跳过清理");
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
    client: &Arc<KichiClient>,
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
            tx,
            req_id,
            &file.abs,
            Some(parent_id.as_str()),
            None,
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
