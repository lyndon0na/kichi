//! 离线任务在界面上实际呈现的状态。

use crate::types::{OfflineTask, TaskPhase};

/// 离线任务在界面上实际呈现的状态。
///
/// 它通常直接来自官方 [`TaskPhase`]，但有一个重要例外：列表请求会附加
/// `reference_resource`，任务产物被删除后官方把该任务覆盖成
/// `PHASE_TYPE_ERROR` + `"File deleted"`。这不是下载失败，不能继续显示成
/// 普通错误，因此在这里派生出 [`TaskStatus::ProductDeleted`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Pending,
    Running,
    Paused,
    Completed,
    Failed,
    ProductDeleted,
    Unknown,
}

impl TaskStatus {
    /// 界面状态是否已经不再变化。产物删除也属于稳定终态。
    pub fn is_settled(self) -> bool {
        matches!(
            self,
            TaskStatus::Completed | TaskStatus::Failed | TaskStatus::ProductDeleted
        )
    }
}

impl OfflineTask {
    /// 把官方阶段与列表覆盖信息合并成用户可见状态。
    ///
    /// 只识别已经确认过的精确标记 `"File deleted"`；`file_not_found` /
    /// `"not found"` 等消息可能指源链接失效，不能据此误判为产物已删除。
    pub fn status(&self) -> TaskStatus {
        if self.phase == TaskPhase::Error
            && self
                .message
                .as_deref()
                .is_some_and(|message| message.trim().eq_ignore_ascii_case("File deleted"))
        {
            return TaskStatus::ProductDeleted;
        }
        match self.phase {
            TaskPhase::Pending => TaskStatus::Pending,
            TaskPhase::Running => TaskStatus::Running,
            TaskPhase::Paused => TaskStatus::Paused,
            TaskPhase::Completed => TaskStatus::Completed,
            TaskPhase::Error => TaskStatus::Failed,
            TaskPhase::Unknown => TaskStatus::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(phase: TaskPhase, message: Option<&str>) -> OfflineTask {
        OfflineTask {
            id: "task".into(),
            name: "file.bin".into(),
            phase,
            progress: 0,
            message: message.map(str::to_owned),
            created_at: None,
            file_id: None,
            file_size: 0,
            kind: None,
            thumbnail_url: String::new(),
            media: false,
            video: false,
        }
    }

    #[test]
    fn deleted_output_has_its_own_terminal_status() {
        let deleted = task(TaskPhase::Error, Some(" File deleted ")).status();
        assert_eq!(deleted, TaskStatus::ProductDeleted);
        assert!(deleted.is_settled());
        assert_eq!(
            task(TaskPhase::Error, Some("File not found")).status(),
            TaskStatus::Failed,
            "源链接失效等普通错误不能冒充产物已删除"
        );
        assert_eq!(task(TaskPhase::Error, None).status(), TaskStatus::Failed);
    }

    #[test]
    fn task_status_follows_official_phases() {
        let cases = [
            (TaskPhase::Pending, TaskStatus::Pending),
            (TaskPhase::Running, TaskStatus::Running),
            (TaskPhase::Paused, TaskStatus::Paused),
            (TaskPhase::Completed, TaskStatus::Completed),
            (TaskPhase::Error, TaskStatus::Failed),
            (TaskPhase::Unknown, TaskStatus::Unknown),
        ];
        for (phase, expected) in cases {
            assert_eq!(task(phase, None).status(), expected);
        }
    }
}
