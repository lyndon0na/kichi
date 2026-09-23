use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::{Duration, Instant};

use kichi_core::download::part_path;
use kichi_core::KichiClient;

use crate::msg::{Msg, QualityOption};

use super::cache::CacheCtl;
use super::discard_part;

/// 预览缓存目录: `~/.cache/kichi/preview`(取不到时退回临时目录)。
pub(super) fn preview_root() -> PathBuf {
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
pub(super) async fn preview_stream(
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
pub(super) async fn preview_qualities(
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

/// 预览缓存是否已命中(UI 侧用于跳过大文件确认)。
pub(crate) fn preview_cached(file_id: &str, name: &str) -> bool {
    preview_cache_path(file_id, name).exists()
}

/// 非媒体预览: 下载到本地缓存(命中缓存则跳过), 再交给系统查看器打开。
pub(super) async fn preview_download(
    client: &KichiClient,
    tx: &Sender<Msg>,
    req_id: u64,
    file_id: String,
    name: String,
    cache: CacheCtl,
    cancel: Arc<AtomicBool>,
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
    // 进度转发限频, 与下载任务一致, 避免高频消息刷 UI。
    let tx2 = tx.clone();
    let mut last_send = Instant::now();
    let mut on_progress = move |total: u64, done: u64| {
        let now = Instant::now();
        if done == 0 || now.duration_since(last_send) >= Duration::from_millis(150) {
            last_send = now;
            let _ = tx2.send(Msg::PreviewProgress {
                req_id,
                total,
                done,
            });
        }
    };
    match client
        .download_to(&link, &dest, Some(cancel.clone()), &mut on_progress)
        .await
    {
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
            if cancel.load(Ordering::Relaxed) {
                // 取消: 丢弃未完成的 .part; UI 已自行清理状态, 无需回消息。
                discard_part(&dest);
                drop(guard);
                return;
            }
            let _ = tx.send(Msg::PreviewFailed {
                req_id,
                what: format!("准备预览文件失败: {e}"),
            });
        }
    }
}
