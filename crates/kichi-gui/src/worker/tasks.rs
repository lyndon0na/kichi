use std::collections::BTreeMap;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use kichi_core::consts::OFFLINE_PHASES;

use crate::msg::Msg;

use super::WorkerState;

/// 离线任务每页条数。
const TASK_PAGE_SIZE: usize = 100;
/// 离线任务快刷节拍: 存在进行中(等待/下载中)任务时, 需要及时反映状态迁移。
pub(super) const TASKS_POLL_ACTIVE: Duration = Duration::from_secs(3);
/// 离线任务慢刷节拍: 无进行中任务时, 仅等待新增/外部变更。
pub(super) const TASKS_POLL_IDLE: Duration = Duration::from_secs(60);

/// 会随时间自行变化的离线任务状态(其余状态只在用户操作时改变)。
fn is_active_phase(phase: &str) -> bool {
    matches!(phase, "PHASE_TYPE_PENDING" | "PHASE_TYPE_RUNNING")
}

pub(super) async fn refresh_tasks(st: &mut WorkerState, tx: &Sender<Msg>) {
    st.last_tasks_poll = Instant::now();
    let Some(client) = st.client.clone() else {
        return;
    };
    let mut buckets: BTreeMap<String, Vec<serde_json::Value>> = BTreeMap::new();
    let mut nexts: BTreeMap<String, Option<String>> = BTreeMap::new();
    let mut active = false;
    let mut active_unknown = false;
    for phase in OFFLINE_PHASES {
        // 保持用户已加载的分页深度(至少 1 页)。
        let want = st.tasks_pages.get(phase).copied().unwrap_or(1).max(1);
        let mut all: Vec<serde_json::Value> = Vec::new();
        let mut token: Option<String> = None;
        let mut fetched = 0usize;
        let mut ok = true;
        for _ in 0..want {
            match client
                .offline_list_phase(phase, TASK_PAGE_SIZE, token.as_deref())
                .await
            {
                Ok(page) => {
                    all.extend(page.tasks);
                    token = page.next_page_token;
                    fetched += 1;
                }
                Err(e) => {
                    tracing::debug!("离线任务[{phase}] 刷新失败: {e}");
                    ok = false;
                    break;
                }
            }
            if token.is_none() {
                break;
            }
        }
        if !ok {
            // 单个分桶失败时保留旧数据(不插入), 连同其旧游标。
            // 进行中分桶失败时状态未知, 维持上一轮的活动判定(避免误降频)。
            if is_active_phase(phase) {
                active_unknown = true;
            }
            continue;
        }
        if is_active_phase(phase) && !all.is_empty() {
            active = true;
        }
        if all.is_empty() {
            st.tasks_pages.remove(phase);
            st.tasks_next.remove(phase);
        } else {
            st.tasks_pages.insert(phase.to_string(), fetched.max(1));
            st.tasks_next.insert(phase.to_string(), token.clone());
        }
        buckets.insert(phase.to_string(), all);
        nexts.insert(phase.to_string(), token);
    }
    st.tasks_active = active || (active_unknown && st.tasks_active);
    let _ = tx.send(Msg::TasksAll {
        buckets,
        next_tokens: nexts,
    });
}

pub(super) async fn load_more_tasks(st: &mut WorkerState, tx: &Sender<Msg>, phase: String) {
    let Some(client) = st.client.clone() else {
        return;
    };
    let Some(token) = st.tasks_next.get(&phase).cloned().flatten() else {
        return;
    };
    // 手动翻页也算一次「刚拉过」, 避免紧接着又触发自动刷新。
    st.last_tasks_poll = Instant::now();
    match client
        .offline_list_phase(&phase, TASK_PAGE_SIZE, Some(&token))
        .await
    {
        Ok(page) => {
            *st.tasks_pages.entry(phase.clone()).or_insert(1) += 1;
            st.tasks_next
                .insert(phase.clone(), page.next_page_token.clone());
            let _ = tx.send(Msg::TasksMore {
                phase,
                tasks: page.tasks,
                next_page_token: page.next_page_token,
            });
        }
        Err(e) => {
            let _ = tx.send(Msg::TasksMoreFailed {
                phase,
                what: format!("加载更多任务失败: {e}"),
            });
        }
    }
}

/// 添加离线下载任务。
pub(super) async fn offline_create(
    st: &WorkerState,
    tx: &Sender<Msg>,
    url: String,
    name: Option<String>,
    parent: Option<String>,
) {
    let Some(client) = &st.client else { return };
    match client
        .offline_create(&url, name.as_deref(), parent.as_deref())
        .await
    {
        Ok(_) => {
            let _ = tx.send(Msg::OfflineCreated);
        }
        Err(e) => {
            let _ = tx.send(Msg::Error {
                what: format!("添加离线下载失败: {e}"),
            });
        }
    }
}

/// 重试离线任务。
pub(super) async fn offline_retry(st: &WorkerState, tx: &Sender<Msg>, task_id: String) {
    let Some(client) = &st.client else { return };
    match client.offline_retry(&task_id).await {
        Ok(_) => {
            let _ = tx.send(Msg::OfflineRetried);
        }
        Err(e) => {
            let _ = tx.send(Msg::Error {
                what: format!("重试任务失败: {e}"),
            });
        }
    }
}

/// 删除离线任务(可选同时删除已落盘的文件)。
pub(super) async fn offline_delete(
    st: &WorkerState,
    tx: &Sender<Msg>,
    task_ids: Vec<String>,
    delete_files: bool,
) {
    let Some(client) = &st.client else { return };
    match client.offline_delete(&task_ids, delete_files).await {
        Ok(_) => {
            let _ = tx.send(Msg::OfflineDeleted);
        }
        Err(e) => {
            let _ = tx.send(Msg::Error {
                what: format!("删除任务失败: {e}"),
            });
        }
    }
}
