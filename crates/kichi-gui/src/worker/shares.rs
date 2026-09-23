use std::collections::HashSet;
use std::time::Duration;

use kichi_core::{Error, KichiClient};

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
