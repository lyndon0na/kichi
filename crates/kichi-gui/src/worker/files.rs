use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;

use crate::msg::Msg;

use super::WorkerState;

/// 加载某目录的文件列表(分页); 非追加加载时作废在跑的缩略图任务。
pub(super) async fn list_files(
    st: &WorkerState,
    tx: &Sender<Msg>,
    parent: Option<String>,
    token: Option<String>,
    append: bool,
    req_id: u64,
) {
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

/// 全盘搜索文件名(客户端递归, 见 kichi-core::client::search_files)。
pub(super) async fn search_files(
    st: &WorkerState,
    tx: &Sender<Msg>,
    keyword: String,
    token: Option<String>,
    append: bool,
    req_id: u64,
) {
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

/// 新建文件夹。
pub(super) async fn create_folder(
    st: &WorkerState,
    tx: &Sender<Msg>,
    name: String,
    parent: Option<String>,
) {
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

/// 重命名文件 / 文件夹。
pub(super) async fn rename(st: &WorkerState, tx: &Sender<Msg>, id: String, name: String) {
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

/// 列出某目录下的子文件夹(「移动到 / 保存到」目录选择用)。
pub(super) async fn list_folders(
    st: &WorkerState,
    tx: &Sender<Msg>,
    parent: Option<String>,
    req_id: u64,
) {
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

/// 移动文件到目标目录。
pub(super) async fn move_files(
    st: &WorkerState,
    tx: &Sender<Msg>,
    ids: Vec<String>,
    dest: Option<String>,
    src: Option<String>,
) {
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

/// 复制文件到目标目录。
pub(super) async fn copy_files(
    st: &WorkerState,
    tx: &Sender<Msg>,
    ids: Vec<String>,
    dest: Option<String>,
) {
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
