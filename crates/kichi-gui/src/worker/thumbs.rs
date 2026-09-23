use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

use eframe::egui;
use kichi_core::download::part_path;
use kichi_core::{Error, KichiClient};
use tokio::sync::Semaphore;

use crate::msg::Msg;

use super::cache::CacheCtl;

/// 缩略图下载并发: 与用户可配的下载并发解耦(缩略图很小, 不该排在
/// 大文件下载后面, 也不该占用用户为下载预留的槽位)。
pub(super) const THUMB_CONCURRENCY: usize = 4;
/// 缩略图下载的最大尝试次数与退避基数(第 n 次失败后睡 n × 基数)。
const THUMB_MAX_ATTEMPTS: usize = 3;
const THUMB_RETRY_BACKOFF: Duration = Duration::from_millis(600);

/// 缩略图缓存目录: `~/.cache/kichi/thumbnails`。
pub(super) fn thumbnail_cache_dir() -> PathBuf {
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

/// 目录已切换 / 刷新时, 在跑的缩略图任务在检查点放弃。
fn thumb_stale(gen: &AtomicU64, my_gen: u64) -> bool {
    gen.load(Ordering::Relaxed) != my_gen
}

/// 服务端下发的 `thumbnail_link` 能不能当下载地址用。
///
/// 对没有缩略图的文件(多为非图片 / 视频), 服务端会下发空串或相对路径之类的值,
/// 连请求都构造不出来(reqwest 报 "builder error")—— 这种失败重试多少次都一样。
fn is_usable_thumb_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

/// 日志用的 URL 片段(空串显示为 `""`, 超长截断), 便于一眼看出服务端发了什么。
fn url_snippet(url: &str) -> String {
    const MAX: usize = 60;
    let mut s: String = url.chars().take(MAX).collect();
    if url.chars().count() > MAX {
        s.push('…');
    }
    s
}

/// 把解码后的图缩到最长边不超过 `max_edge`: 服务端缩略图(实测 720×405)远大于
/// 卡片所需, 上传 GPU 前先降采样, 显存与上传量都按输出尺寸算。
/// 已经足够小的图原样返回(不放大)。
fn fit_within_max_edge(img: image::DynamicImage, max_edge: u32) -> image::DynamicImage {
    let (w, h) = (img.width(), img.height());
    let longest = w.max(h);
    if longest <= max_edge {
        return img;
    }
    let scale = max_edge as f64 / longest as f64;
    let nw = ((w as f64 * scale).round() as u32).max(1);
    let nh = ((h as f64 * scale).round() as u32).max(1);
    img.thumbnail(nw, nh)
}

/// 加载缩略图: 先检查磁盘缓存, 未命中则从 URL 下载(有限次退避重试),
/// 解码、按 `max_edge` 降采样为 RGBA 后发送给 UI。
///
/// 失败一律回 `Msg::ThumbnailFailed`, 否则请求方的在途登记会悬空, 该文件
/// 本次会话再也不会被请求(网格留空位); 目录代数过期则静默放弃, 不回包。
#[allow(clippy::too_many_arguments)]
pub(super) async fn load_thumbnail(
    client: &KichiClient,
    tx: &Sender<Msg>,
    cache: &CacheCtl,
    sem: Arc<Semaphore>,
    gen: Arc<AtomicU64>,
    my_gen: u64,
    file_id: String,
    url: String,
    max_edge: u32,
) {
    if !is_usable_thumb_url(&url) {
        // 链接本身不可用(见 is_usable_thumb_url): 对这类文件来说「没有缩略图」是
        // 正常状态而非故障, 所以不下载、不重试, 回退类型图标即可。要查是哪些文件,
        // 用 KICHI_LOG=kichi_gui=trace 跑一次。
        tracing::trace!(
            "缩略图链接不可用 {file_id} (长度 {}): {:?}",
            url.len(),
            url_snippet(&url)
        );
        let _ = tx.send(Msg::ThumbnailFailed { file_id });
        return;
    }

    let dest = thumbnail_cache_path(&file_id);
    let mut _guard = None;

    if dest.exists() {
        // 命中缓存: 刷新 mtime, 让 LRU 知道它刚被用过。
        crate::cache::touch(&dest);
    } else {
        // 缩略图不占用户下载槽位(不该排在大文件下载后面), 但要限并发,
        // 否则大目录下会同时开出成百上千个连接。
        let Ok(_permit) = sem.acquire_owned().await else {
            return;
        };
        // 排队等槽位期间可能已被别的任务写好。
        if !dest.exists() {
            _guard = Some(cache.mark_in_use(&[dest.clone(), part_path(&dest)]));
            let mut ok = false;
            let mut last: Option<(usize, Error)> = None;
            for attempt in 1..=THUMB_MAX_ATTEMPTS {
                if thumb_stale(&gen, my_gen) {
                    return;
                }
                match client.download_thumbnail(&url, &dest).await {
                    Ok(()) => {
                        ok = true;
                        break;
                    }
                    Err(e) => {
                        // 永久性失败(404 / 403 之类)重试多少次都一样, 直接收工。
                        let transient = e.is_transient();
                        last = Some((attempt, e));
                        if !transient || attempt == THUMB_MAX_ATTEMPTS {
                            break;
                        }
                        tokio::time::sleep(THUMB_RETRY_BACKOFF * attempt as u32).await;
                    }
                }
            }
            if !ok {
                // 已在下载过程中切了目录就没必要再回失败(UI 侧那份登记随目录切换清掉了)。
                if !thumb_stale(&gen, my_gen) {
                    // 每个文件只在终态记一行: 逐次尝试都记会让一个坏链接刷 3 行日志。
                    // 带 Debug 反查错误来源链(超时 / 连接被重置 / DNS 等)。
                    let what = last
                        .map(|(n, e)| format!("已试 {n} 次: {e:?}"))
                        .unwrap_or_default();
                    tracing::debug!(
                        "缩略图下载失败 {file_id} (长度 {}): {what} {:?}",
                        url.len(),
                        url_snippet(&url)
                    );
                    let _ = tx.send(Msg::ThumbnailFailed { file_id });
                }
                return;
            }
            let size = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
            cache.note_write(size);
        }
    }

    if thumb_stale(&gen, my_gen) {
        return;
    }

    // 解码期间保持 _guard(缩略图是「写完即读」), 避免文件刚落地就被并发淘汰删掉。
    let read_path = dest.clone();
    let result = tokio::task::spawn_blocking(move || -> Option<(u32, u32, Vec<egui::Color32>)> {
        let data = std::fs::read(&read_path).ok()?;
        let img = image::load_from_memory(&data).ok()?;
        let rgba = fit_within_max_edge(img, max_edge).to_rgba8();
        let (w, h) = rgba.dimensions();
        let pixels: Vec<egui::Color32> = rgba
            .pixels()
            .map(|p| egui::Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3]))
            .collect();
        Some((w, h, pixels))
    })
    .await;

    if thumb_stale(&gen, my_gen) {
        return;
    }
    match result {
        Ok(Some((width, height, pixels))) => {
            let _ = tx.send(Msg::ThumbnailReady {
                file_id,
                width,
                height,
                pixels,
            });
        }
        Ok(None) => {
            // 解码失败通常说明磁盘上的缓存文件已损坏(旧版本的非原子写入会留下
            // 截断文件): 删掉它, 让「刷新后重试」能真正重新下载, 而不是永远失败。
            let _ = std::fs::remove_file(&dest);
            tracing::debug!("缩略图解码失败 {file_id}, 已丢弃缓存文件");
            let _ = tx.send(Msg::ThumbnailFailed { file_id });
        }
        Err(e) => {
            let _ = std::fs::remove_file(&dest);
            tracing::debug!("缩略图解码任务失败 {file_id}: {e}, 已丢弃缓存文件");
            let _ = tx.send(Msg::ThumbnailFailed { file_id });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_within_max_edge_downscales_and_keeps_aspect() {
        // 服务端实测 720×405, 按 272 缩到 272×153(四舍五入)。
        let img = image::DynamicImage::new_rgba8(720, 405);
        let out = fit_within_max_edge(img, 272);
        assert_eq!((out.width(), out.height()), (272, 153));
    }

    #[test]
    fn fit_within_max_edge_uses_longest_side_for_portrait() {
        let img = image::DynamicImage::new_rgba8(405, 720);
        let out = fit_within_max_edge(img, 272);
        assert_eq!((out.width(), out.height()), (153, 272));
    }

    #[test]
    fn fit_within_max_edge_does_not_upscale_small_images() {
        let img = image::DynamicImage::new_rgba8(136, 76);
        let out = fit_within_max_edge(img, 272);
        assert_eq!((out.width(), out.height()), (136, 76));
    }
}
