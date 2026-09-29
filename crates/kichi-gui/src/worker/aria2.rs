//! aria2 推送管线: 解析限时直链 → `aria2.addUri`, 独立小并发(不占下载 Gate)。
//!
//! 与三条长任务管线的区别: 推送本身不传字节, 只是「解析直链 + 一次 RPC」, 秒级完成;
//! 真正下载由 aria2 负责。直链限时是已知代价 —— 排队过久 / 下载太慢时会在 aria2
//! 侧过期失败, 需重新推送(不做 Kichi 代理中转)。

use std::path::Path;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::{Duration, Instant};

use kichi_core::aria2::{AddUriOptions, Aria2Client};
use kichi_core::{Error, KichiClient};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::msg::{Aria2Config, Msg};

use super::download::join_local_path;
use super::{download_backoff, WorkerState};

/// 推送并发: 与用户可配的下载并发解耦(推送只是解析直链 + RPC, 不占下载槽位)。
const ARIA2_CONCURRENCY: usize = 4;
/// 解析限时直链的尝试次数(批量推送里单个文件解析失败不该拖垮整批)。
const LINK_ATTEMPTS: u32 = 2;
/// 进度消息节流(与下载 / 预览一致): 大批量推送不必每完成一个文件就刷一次 UI。
const PROGRESS_THROTTLE: Duration = Duration::from_millis(150);

/// 一个待推送的文件: 云端相对目录组件(不含被推送目录自身名)与文件名。
struct Target {
    file_id: String,
    rel: Vec<String>,
    name: String,
}

/// 派发一次 aria2 推送: 整目录先扫描, 再逐文件解析直链交给 aria2。
pub(super) async fn push(
    st: &WorkerState,
    tx: &Sender<Msg>,
    config: Aria2Config,
    label: String,
    files: Vec<(String, String)>,
    folders: Vec<(String, String)>,
) {
    let Some(client) = st.client.clone() else {
        return;
    };
    let tx = tx.clone();
    tokio::spawn(async move {
        // 1) 目标清单: 直接推送的文件平铺在基目录下, 整目录先递归扫描保留云端层级。
        let mut targets: Vec<Target> = Vec::new();
        for (file_id, name) in files {
            targets.push(Target {
                file_id,
                rel: Vec::new(),
                name,
            });
        }
        for (folder_id, name) in folders {
            let walk = match client.walk_folder(&folder_id).await {
                Ok(w) => w,
                Err(e) => {
                    tracing::warn!("aria2 推送扫描目录失败「{name}」: {e}");
                    let _ = tx.send(Msg::Aria2Failed {
                        what: format!("扫描目录「{name}」失败: {e}"),
                    });
                    return;
                }
            };
            // 被推送目录自身的名称作为层级根(walk 的相对路径不含目录自身)。
            let mut root = Vec::new();
            if let Some(safe) = crate::format::safe_file_name(&name) {
                root.push(safe);
            }
            for (rel, f) in walk.files {
                let mut parts = root.clone();
                parts.extend(rel);
                targets.push(Target {
                    file_id: f.id,
                    rel: parts,
                    name: f.name,
                });
            }
        }
        if targets.is_empty() {
            let _ = tx.send(Msg::Aria2Pushed {
                label,
                ok: 0,
                failed: 0,
                first_error: String::new(),
            });
            return;
        }
        let total = targets.len() as u32;

        // 2) 目标基目录: 设置里显式指定优先; 留空且要拼云端层级时读 aria2 的全局 dir
        //    (连不上就在推送前整批失败, 不逐个文件重复报同一个错)。
        let aria = Arc::new(Aria2Client::new(config.rpc_url, config.secret));
        let mut base = config.dir;
        if base.is_empty() && targets.iter().any(|t| !t.rel.is_empty()) {
            match aria.global_dir().await {
                Ok(dir) => base = dir,
                Err(e) => {
                    let _ = tx.send(Msg::Aria2Failed {
                        what: format!("读取 aria2 下载目录失败: {e}"),
                    });
                    return;
                }
            }
        }
        let base: Arc<str> = Arc::from(base.as_str());

        // 3) 逐文件推送(独立小闸并发, 与下载 / 缩略图解耦), 如实统计成败并回传首个失败原因。
        let sem = Arc::new(Semaphore::new(ARIA2_CONCURRENCY));
        let mut set: JoinSet<Result<(), String>> = JoinSet::new();
        for t in targets {
            let client = client.clone();
            let aria = aria.clone();
            let sem = sem.clone();
            let base = base.clone();
            set.spawn(async move {
                let Ok(_permit) = sem.acquire_owned().await else {
                    return Err("推送已中止".to_string());
                };
                push_one(&client, &aria, &base, &t).await
            });
        }

        let mut done: u32 = 0;
        let mut ok: u32 = 0;
        let mut failed: u32 = 0;
        let mut first_error = String::new();
        let mut last_sent = Instant::now();
        while let Some(joined) = set.join_next().await {
            match joined.unwrap_or_else(|e| Err(format!("推送任务异常: {e}"))) {
                Ok(()) => ok += 1,
                Err(e) => {
                    failed += 1;
                    if first_error.is_empty() {
                        first_error = e;
                    }
                }
            }
            done += 1;
            if last_sent.elapsed() >= PROGRESS_THROTTLE {
                last_sent = Instant::now();
                let _ = tx.send(Msg::Aria2Progress {
                    label: label.clone(),
                    done,
                    total,
                });
            }
        }
        tracing::info!("aria2 推送结束「{label}」: 成功 {ok} / 失败 {failed} (共 {total})");
        let _ = tx.send(Msg::Aria2Pushed {
            label,
            ok,
            failed,
            first_error,
        });
    });
}

/// 测试 aria2 连接(设置页「测试连接」): 只做一次 `aria2.getVersion`。
pub(super) fn test_connection(tx: &Sender<Msg>, config: Aria2Config) {
    let tx = tx.clone();
    tokio::spawn(async move {
        let aria = Aria2Client::new(config.rpc_url, config.secret);
        match aria.version().await {
            Ok(version) => {
                let _ = tx.send(Msg::Aria2TestOk { version });
            }
            Err(e) => {
                let _ = tx.send(Msg::Aria2TestFailed {
                    what: e.to_string(),
                });
            }
        }
    });
}

/// 推送单个文件: 解析限时直链 → 取服务端接受的请求头 → `aria2.addUri`。
async fn push_one(
    client: &KichiClient,
    aria: &Aria2Client,
    base: &str,
    t: &Target,
) -> Result<(), String> {
    let link = resolve_link(client, &t.file_id).await?;
    // 服务端要求的请求头(UA / X-Device-Id / 必要时 Bearer)原样交给 aria2,
    // 使 aria2 的请求与内置下载一致; 实测 header 数组会覆盖 aria2 全局 UA。
    let headers = client.stream_headers(&link.url).await;
    let out = crate::format::safe_file_name(&link.name)
        .or_else(|| crate::format::safe_file_name(&t.name))
        .unwrap_or_else(|| "download".to_string());
    let opts = AddUriOptions {
        out,
        dir: target_dir(base, &t.rel),
        headers,
    };
    aria.add_uri(&link.url, &opts).await.map_err(|e| {
        tracing::debug!("aria2 推送失败「{}」: {e}", t.name);
        e.to_string()
    })?;
    Ok(())
}

/// 解析限时直链; 瞬时错误做有限次退避重试(直链解析走 API, 与下载同一套路)。
async fn resolve_link(
    client: &KichiClient,
    file_id: &str,
) -> Result<kichi_core::download::DownloadLink, String> {
    let mut last = Error::msg("未知错误");
    for attempt in 1..=LINK_ATTEMPTS {
        match client.file_download_link(file_id).await {
            Ok(link) => return Ok(link),
            Err(e) => {
                let transient = e.is_transient();
                last = e;
                if !transient || attempt == LINK_ATTEMPTS {
                    break;
                }
                tokio::time::sleep(download_backoff(attempt - 1)).await;
            }
        }
    }
    Err(format!("解析直链失败: {last}"))
}

/// 目标目录: 基目录 + 云端相对层级(逐级净化); 无层级时用基目录本身。
/// 返回空串表示不下发 `dir`, 由 aria2 用它自己的全局下载目录。
fn target_dir(base: &str, rel: &[String]) -> String {
    if rel.is_empty() {
        base.to_string()
    } else {
        join_local_path(Path::new(base), rel)
            .to_string_lossy()
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_dir_joins_cloud_hierarchy_under_base() {
        // 留空基目录 = 交给 aria2 的全局 dir(单文件)。
        assert_eq!(target_dir("", &[]), "");
        assert_eq!(target_dir("/dl", &[]), "/dl");
        assert_eq!(
            target_dir("/dl", &["剧集".to_string(), "S1".to_string()]),
            "/dl/剧集/S1"
        );
        // 基目录留空但需要拼层级(整目录推送): 退化为纯相对层级。
        assert_eq!(target_dir("", &["A".to_string()]), "A");
    }

    #[test]
    fn target_dir_sanitizes_components() {
        assert_eq!(target_dir("/dl", &["../evil".to_string()]), "/dl/evil");
    }
}
