use std::sync::mpsc::Sender;

use crate::msg::Msg;

use super::WorkerState;

/// 把文件移入回收站。
pub(super) async fn trash_files(st: &WorkerState, tx: &Sender<Msg>, ids: Vec<String>) {
    let Some(client) = &st.client else { return };
    match client.batch_trash(&ids).await {
        Ok(_) => {
            let _ = tx.send(Msg::Trashed);
        }
        Err(e) => {
            let _ = tx.send(Msg::Error {
                what: format!("删除失败: {e}"),
            });
        }
    }
}

/// 加载回收站列表(分页, 必须走 parent_id=* 否则服务端按当前目录过滤)。
pub(super) async fn list_trash(
    st: &WorkerState,
    tx: &Sender<Msg>,
    token: Option<String>,
    append: bool,
    req_id: u64,
) {
    let Some(client) = &st.client else { return };
    match client.trash_list(100, token.as_deref()).await {
        Ok(list) => {
            let _ = tx.send(Msg::TrashList {
                req_id,
                append,
                list,
            });
        }
        Err(e) => {
            let _ = tx.send(Msg::TrashFailed {
                what: format!("加载回收站失败: {e}"),
            });
        }
    }
}

/// 从回收站还原文件。
pub(super) async fn untrash_files(st: &WorkerState, tx: &Sender<Msg>, ids: Vec<String>) {
    let Some(client) = &st.client else { return };
    match client.batch_untrash(&ids).await {
        Ok(_) => {
            let _ = tx.send(Msg::TrashRestored { ids });
        }
        Err(e) => {
            let _ = tx.send(Msg::Error {
                what: format!("还原失败: {e}"),
            });
        }
    }
}

/// 彻底删除回收站中的文件。
pub(super) async fn delete_trash_files(st: &WorkerState, tx: &Sender<Msg>, ids: Vec<String>) {
    let Some(client) = &st.client else { return };
    match client.batch_delete(&ids).await {
        Ok(_) => {
            let _ = tx.send(Msg::TrashDeleted { ids });
        }
        Err(e) => {
            let _ = tx.send(Msg::Error {
                what: format!("彻底删除失败: {e}"),
            });
        }
    }
}

/// 清空回收站。
pub(super) async fn empty_trash(st: &WorkerState, tx: &Sender<Msg>) {
    let Some(client) = &st.client else { return };
    match client.empty_trash().await {
        Ok(()) => {
            let _ = tx.send(Msg::TrashEmptied);
        }
        Err(e) => {
            let _ = tx.send(Msg::Error {
                what: format!("清空回收站失败: {e}"),
            });
        }
    }
}
