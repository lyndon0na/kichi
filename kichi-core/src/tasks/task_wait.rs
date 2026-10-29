//! 批量写操作的异步任务轮询。

use std::time::{Duration, Instant};

use crate::drive::DriveClient;
use crate::types::TaskPhase;
use crate::Error;

use super::{phase_from, TaskDto};

const TASK_POLL_INTERVAL: Duration = Duration::from_millis(500);
const TASK_WAIT_LIMIT: Duration = Duration::from_secs(120);

impl DriveClient {
    /// 等待批量写操作产生的后台任务结束。
    ///
    /// `batchTrash` / `batchUntrash` / `batchDelete` / `trash:empty` 都先返回
    /// `task_id`，真正执行是异步的。必须等到 `PHASE_TYPE_COMPLETE` 再刷新列表，
    /// 否则界面会把“任务已接收”误当成“数据已落库”。
    pub(crate) fn wait_task(&self, id: &str) -> Result<(), Error> {
        let id = id.trim();
        if id.is_empty() {
            return Err(Error::ApiChanged(
                "后台操作响应里缺少 task_id，无法确认是否完成".into(),
            ));
        }

        let started = Instant::now();
        loop {
            if started.elapsed() >= TASK_WAIT_LIMIT {
                return Err(Error::Other(format!(
                    "等待 PikPak 后台任务完成超时（{} 秒）",
                    TASK_WAIT_LIMIT.as_secs()
                )));
            }
            std::thread::sleep(TASK_POLL_INTERVAL);

            let value: serde_json::Value = self.get(&self.config().task_url(id))?;
            let task = task_from_value(value)?;
            match task_poll(&task.phase, task.message.as_deref()) {
                TaskPoll::Completed => return Ok(()),
                TaskPoll::Running => {}
                TaskPoll::Failed(message) => return Err(Error::Other(message)),
                TaskPoll::Invalid(message) => return Err(Error::ApiChanged(message)),
            }
        }
    }
}

fn task_from_value(value: serde_json::Value) -> Result<TaskDto, Error> {
    let task_value = value.get("task").cloned().unwrap_or(value);
    serde_json::from_value(task_value)
        .map_err(|err| Error::ApiChanged(format!("后台任务响应无法识别：{err}")))
}

#[derive(Debug, PartialEq, Eq)]
enum TaskPoll {
    Completed,
    Running,
    Failed(String),
    Invalid(String),
}

/// 把任务阶段转成轮询决定。空阶段说明响应形状不对，立即报错；
/// 暂停 / 未知阶段仍继续等，避免把暂时状态误判为永久失败。
fn task_poll(phase: &str, message: Option<&str>) -> TaskPoll {
    match phase_from(phase) {
        TaskPhase::Completed => TaskPoll::Completed,
        TaskPhase::Error => {
            let reason = message
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .unwrap_or("PikPak 未提供失败原因");
            TaskPoll::Failed(format!("PikPak 后台任务失败：{reason}"))
        }
        TaskPhase::Unknown if phase.trim().is_empty() => {
            TaskPoll::Invalid("后台任务响应里没有 phase".into())
        }
        TaskPhase::Pending | TaskPhase::Running | TaskPhase::Paused | TaskPhase::Unknown => {
            TaskPoll::Running
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_poll_requires_completion_and_surfaces_errors() {
        assert_eq!(task_poll("PHASE_TYPE_COMPLETE", None), TaskPoll::Completed);
        assert_eq!(task_poll("PHASE_TYPE_RUNNING", None), TaskPoll::Running);
        assert_eq!(task_poll("PHASE_TYPE_PAUSED", None), TaskPoll::Running);
        assert_eq!(task_poll("PHASE_TYPE_UNKNOW", None), TaskPoll::Running);
        assert_eq!(
            task_poll("PHASE_TYPE_ERROR", Some(" disk full ")),
            TaskPoll::Failed("PikPak 后台任务失败：disk full".into())
        );
        assert!(matches!(
            task_poll("", None),
            TaskPoll::Invalid(message) if message.contains("phase")
        ));
    }

    #[test]
    fn task_response_accepts_bare_and_wrapped_objects() {
        let bare = serde_json::json!({
            "id": "task-1",
            "phase": "PHASE_TYPE_COMPLETE"
        });
        let wrapped = serde_json::json!({
            "task": {
                "id": "task-2",
                "phase": "PHASE_TYPE_RUNNING"
            }
        });
        let bare = task_from_value(bare).expect("裸任务对象应可解析");
        let wrapped = task_from_value(wrapped).expect("task 包装对象应可解析");
        assert_eq!(bare.phase, "PHASE_TYPE_COMPLETE");
        assert_eq!(wrapped.phase, "PHASE_TYPE_RUNNING");
    }

    #[test]
    fn empty_task_id_is_rejected_before_network() {
        let config = crate::Config::default();
        let client = DriveClient::new(&config, "0123456789abcdef0123456789abcdef", "", "")
            .expect("客户端应可构造");
        assert!(matches!(
            client.wait_task("  "),
            Err(Error::ApiChanged(message)) if message.contains("task_id")
        ));
    }
}
