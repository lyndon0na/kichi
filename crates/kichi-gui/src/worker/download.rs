use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kichi_core::{Error, KichiClient};

use crate::msg::{FolderItem, Msg};

use super::{discard_part, download_backoff, WorkerState};

/// 注册一个下载任务并 spawn 后台协程。目标文件名在此处净化、去重并占位,
/// 因此即便任务在并发信号量上排队, 同名任务也会分到不同的文件。
pub(super) async fn spawn_download(
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
pub(super) async fn spawn_folder_download(
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
pub(super) fn join_local_path(base: &Path, rel: &[String]) -> PathBuf {
    let mut p = base.to_path_buf();
    for c in rel {
        p.push(crate::format::safe_file_name(c).unwrap_or_else(|| "download".to_string()));
    }
    p
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
}
