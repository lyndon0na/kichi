//! 离线任务的会话门面：提交 / 拉取 / 删除。
//!
//! 本模块只做「一次请求的编排」，不含任何定时或线程逻辑；
//! 轮询节奏属于界面侧的调度，放在 `cxxqt_object::tasks`。
//! 全部函数阻塞，调用方负责丢到后台线程。

use kichi_core::OfflineTask;

use super::{Failure, Session};

/// 任务列表最多翻 20 页（10,000 条），防止异常分页把内存打满。
const MAX_TASK_PAGES: usize = 20;
const TASK_PAGE_SIZE: usize = kichi_core::tasks::TASK_LIST_LIMIT as usize;

/// 批量删除任务记录的结果。
///
/// 遇到首个失败就停止，因此 `deleted` 可能大于 0 且小于 `total`。
pub struct BatchDeleteOutcome {
    pub total: usize,
    pub deleted: usize,
    pub failure: Option<Failure>,
}

impl Session {
    /// 提交离线下载（磁力 / ed2k / 普通 URL）。阻塞。
    ///
    /// `parent_id = None` 使用官方默认下载目录；`Some("")` 表示根目录。
    pub fn add_offline_task(
        &self,
        url: &str,
        parent_id: Option<&str>,
    ) -> Result<OfflineTask, Failure> {
        let client = self.drive()?;
        client
            .create_offline_task(url, parent_id)
            .map_err(Failure::from)
    }

    /// 拉取全部离线任务（自动翻页；阻塞）。
    pub fn offline_tasks(&self) -> Result<Vec<OfflineTask>, Failure> {
        let client = self.drive()?;
        let mut tasks = Vec::new();
        let mut page_token: Option<String> = None;
        for _ in 0..MAX_TASK_PAGES {
            let (mut page, next) = client
                .list_offline_tasks(page_token.as_deref())
                .map_err(Failure::from)?;
            tasks.append(&mut page);
            match next {
                Some(token) => page_token = Some(token),
                None => return Ok(tasks),
            }
        }
        Err(Failure::Other(format!(
            "离线任务超过 {} 条，暂不支持完整显示",
            MAX_TASK_PAGES * TASK_PAGE_SIZE
        )))
    }

    /// 删除任务记录（云端文件保留）。阻塞。
    pub fn delete_offline_task(&self, id: &str) -> Result<(), Failure> {
        let client = self.drive()?;
        client.delete_offline_task(id, false).map_err(Failure::from)
    }

    /// 逐条删除一批任务记录（云端文件保留）。阻塞。
    ///
    /// 不假定 `task_ids` 支持逗号分隔；每条都走已验证的单 id 删除接口。
    /// 遇到首个失败立即停止，已经成功的删除不会回滚。
    pub fn delete_offline_tasks(&self, ids: &[String]) -> BatchDeleteOutcome {
        let mut deleted = 0;
        for id in ids {
            match self.delete_offline_task(id) {
                Ok(()) => deleted += 1,
                Err(failure) => {
                    return BatchDeleteOutcome {
                        total: ids.len(),
                        deleted,
                        failure: Some(failure),
                    };
                }
            }
        }
        BatchDeleteOutcome {
            total: ids.len(),
            deleted,
            failure: None,
        }
    }
}

/// 是否还有任务在变化（决定轮询频率）。
pub fn has_active(tasks: &[OfflineTask]) -> bool {
    tasks.iter().any(|task| !task.status().is_settled())
}
