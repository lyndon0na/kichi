use std::collections::HashSet;
use std::sync::mpsc::Sender;
use std::time::Duration;

use kichi_core::{Error, KichiClient};

use crate::msg::Msg;

use super::WorkerState;

/// 定位服务端转存暂存目录(「转存自分享」/ "Pack From Shared")。
/// 优先使用持久化的目录 ID; ID 失效(如用户删除后服务端重建)时回退到名称匹配,
/// 并把新 ID 写回缓存。服务端从未产生过该目录时返回 None。
pub(super) async fn find_pack_folder(
    client: &KichiClient,
) -> Result<Option<kichi_core::types::File>, Error> {
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
pub(super) async fn snapshot_pack_folder(client: &KichiClient) -> Result<HashSet<String>, Error> {
    let Some(folder) = find_pack_folder(client).await? else {
        return Ok(HashSet::new());
    };
    let pack_list = client.file_list(Some(&folder.id), 100, None).await?;
    Ok(pack_list.files.iter().map(|f| f.id.clone()).collect())
}

/// 转存后自动移动: 等待服务端写入完成, 找出「转存自分享」中新增的文件并移动到目标目录。
/// 使用重试机制轮询等待服务端同步, 而非固定 sleep。
pub(super) async fn move_new_files(
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

/// 创建分享链接。
pub(super) async fn create_share(
    st: &WorkerState,
    tx: &Sender<Msg>,
    file_ids: Vec<String>,
    expiration_days: i64,
    need_password: bool,
    label: String,
) {
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

/// 加载我的分享列表(分页)。
pub(super) async fn list_shares(
    st: &WorkerState,
    tx: &Sender<Msg>,
    token: Option<String>,
    append: bool,
    req_id: u64,
) {
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

/// 取消分享。
pub(super) async fn delete_shares(st: &WorkerState, tx: &Sender<Msg>, ids: Vec<String>) {
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

/// 解析分享链接(取标题、文件列表与下一页游标)。
pub(super) async fn resolve_share(
    st: &WorkerState,
    tx: &Sender<Msg>,
    share_id: String,
    pass_code: String,
) {
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

/// 加载分享内文件的下一页。
pub(super) async fn load_more_share_files(
    st: &WorkerState,
    tx: &Sender<Msg>,
    share_id: String,
    pass_code_token: String,
    page_token: String,
) {
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

/// 转存分享文件; 指定目标目录时只把本次新增的文件从暂存目录移动过去。
pub(super) async fn save_share(
    st: &WorkerState,
    tx: &Sender<Msg>,
    share_id: String,
    pass_code_token: String,
    file_ids: Vec<String>,
    dest: Option<String>,
) {
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

/// 重试自动移动: 把「转存自分享」中的全部文件移动到目标目录。
pub(super) async fn retry_move_share(st: &WorkerState, tx: &Sender<Msg>, dest: String) {
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
