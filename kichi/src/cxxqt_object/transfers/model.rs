//! 传输记录与批次种子类型。

use std::path::PathBuf;

use kichi_core::{EntryKind, FileEntry, UploadStage};

use super::*;
use crate::cxxqt_object::transfers_rate::{NoteState, SpeedMeter};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransferState {
    /// 排队中，还没轮到
    Queued,
    /// 轮到了、正在读目录结构（只有文件夹批次的首次展开）
    Preparing,
    Running,
    /// 用户已请求暂停，下载线程正在收尾
    Pausing,
    /// 已暂停：保留断点，不占调度槽，可继续或取消
    Paused,
    Done,
    Failed,
    Cancelled,
    /// 上次退出时还没跑完（读回历史时标上）；不能取消、也不算结束，
    /// 但可以「重试」——重启后不自动续跑。
    Interrupted,
}

impl TransferState {
    /// 是否占用调度槽 / 计入侧栏「传输中」。
    ///
    /// **不含** `Paused`：暂停任务不占调度槽，也不该让侧栏一直显示在传输。
    /// **不含** `Interrupted`：那一轮下载已经不存在了。
    ///
    /// 写成显式 `match`（不用 `matches!`）：将来加状态时编译器会提醒这里漏了分支。
    pub(crate) fn is_active(self) -> bool {
        match self {
            Self::Queued | Self::Preparing | Self::Running | Self::Pausing => true,
            Self::Paused | Self::Done | Self::Failed | Self::Cancelled | Self::Interrupted => false,
        }
    }

    /// 是否还能被用户取消。
    ///
    /// `Paused` 不占调度槽，但仍保留断点并允许显式取消（取消时要删 part）。
    pub(crate) fn is_cancellable(self) -> bool {
        match self {
            Self::Queued | Self::Preparing | Self::Running | Self::Pausing | Self::Paused => true,
            Self::Done | Self::Failed | Self::Cancelled | Self::Interrupted => false,
        }
    }

    /// 能否被「清空已完成记录」/ 按上限淘汰清掉。
    ///
    /// **不**含 `Interrupted`：已中断的行是用户要看、要重试的东西，只有
    /// 「清空全部记录」和清理规则能删它。
    pub(crate) fn is_finished(self) -> bool {
        match self {
            Self::Done | Self::Failed | Self::Cancelled => true,
            Self::Queued
            | Self::Preparing
            | Self::Running
            | Self::Pausing
            | Self::Paused
            | Self::Interrupted => false,
        }
    }

    #[cfg(test)]
    pub(crate) fn text(self) -> &'static str {
        self.text_for(TransferDirection::Download)
    }

    /// 进行中的文案按方向区分，其余状态共用。
    pub(crate) fn text_for(self, direction: TransferDirection) -> &'static str {
        match self {
            Self::Queued => "排队中",
            Self::Preparing => "准备中",
            Self::Running => match direction {
                TransferDirection::Download => "下载中",
                TransferDirection::Upload => "上传中",
            },
            Self::Pausing => "正在暂停",
            Self::Paused => "已暂停",
            Self::Done => "已完成",
            Self::Failed => "失败",
            Self::Cancelled => "已取消",
            Self::Interrupted => "已中断",
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Preparing => "preparing",
            Self::Running => "running",
            Self::Pausing => "pausing",
            Self::Paused => "paused",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
        }
    }
}

/// 记录属于本地下载还是云端上传。旧记录没有这个字段，读回时按下载处理。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum TransferDirection {
    #[default]
    Download,
    Upload,
}

impl TransferDirection {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Download => "download",
            Self::Upload => "upload",
        }
    }

    pub(crate) fn from_name(name: &str) -> Self {
        match name {
            "upload" => Self::Upload,
            _ => Self::Download,
        }
    }
}

/// 上传记录额外需要的重试目标；下载记录为 `None`。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct UploadJob {
    pub(crate) local_path: PathBuf,
    pub(crate) parent_id: String,
    pub(crate) parent_name: String,
    /// 上传文件夹批次：根目录创建后的云端 id；单文件上传为空。
    pub(crate) batch_root_id: String,
    /// 上传文件夹子文件：目录链就绪后的直接父目录 id；未就绪为空。
    pub(crate) cloud_parent_id: String,
}

/// 上传文件夹批次里已经创建成功的云端目录。
///
/// `path` 是相对目标目录的完整层级，包含本地选中的根文件夹名。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UploadDir {
    pub(crate) path: Vec<String>,
    pub(crate) id: String,
}

/// 上传进行中阶段。只存在内存里，重启后的活动记录已经是「已中断」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UploadProgressState {
    pub(crate) stage: UploadStage,
    /// 界面显示的实际进度（含在途分片）。
    pub(crate) bytes: u64,
    /// 已完成分片字节；只用来推进断点与传输历史，不能把在途字节写进去。
    pub(crate) completed_bytes: Option<u64>,
    pub(crate) total: u64,
}

/// 传输记录的重试种子：一行要下载什么、写到哪里。
#[derive(Debug, Clone)]
pub(crate) struct TransferJob {
    /// 云端条目 id：文件行 = 文件 id，文件夹总账行 = 文件夹 id
    pub(crate) id: String,
    /// 裸名字（不含 sub_dir）
    pub(crate) name: String,
    pub(crate) kind: EntryKind,
    /// 官方标注的大小（0 = 未知）
    pub(crate) size: u64,
    /// 列表里的审核标记（详情没下发 audit 时用来跳过无用的 5 秒重试）
    pub(crate) audit_flagged: bool,
    /// 相对下载目录的层级；单文件 / 文件夹总账行为空
    pub(crate) sub_dir: Vec<String>,
    /// 入队时的下载目录快照
    pub(crate) dir: PathBuf,
    pub(crate) direction: TransferDirection,
    pub(crate) upload: Option<UploadJob>,
}

impl TransferJob {
    /// 从当前目录 / 搜索结果里的条目建种子（普通「下载」入口）。
    pub(crate) fn from_entry(entry: &FileEntry, dir: PathBuf) -> Self {
        Self {
            id: entry.id.clone(),
            name: entry.name.clone(),
            kind: entry.kind,
            size: entry.size,
            audit_flagged: entry.audit_flagged,
            sub_dir: Vec::new(),
            dir,
            direction: TransferDirection::Download,
            upload: None,
        }
    }

    /// 界面显示的路径：`剧集/第 1 集.mkv`；单文件就是裸名字。
    pub(crate) fn display_name(&self) -> String {
        if self.sub_dir.is_empty() {
            self.name.clone()
        } else {
            format!("{}/{}", self.sub_dir.join("/"), self.name)
        }
    }

    pub(crate) fn is_upload(&self) -> bool {
        self.direction == TransferDirection::Upload
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Transfer {
    /// 批次序号（进程内自增）
    pub(crate) batch: u64,
    /// 批内顺序，决定同批次的显示顺序
    pub(crate) index: usize,
    /// 文件夹批次展开出来的子文件行（只在父行展开时进入统一列表）
    pub(crate) child: bool,
    /// 要重下所需的一切（id / 裸名 / 类型 / 大小 / 审核标记 / 子目录 / 目录快照）
    pub(crate) job: TransferJob,
    /// 来源条目 id：同一批的每一行都一样，QML 据此定位到批次
    pub(crate) source_id: String,
    pub(crate) state: TransferState,
    pub(crate) bytes: u64,
    pub(crate) total: Option<u64>,
    /// 失败 / 取消原因（完成行的说明由 path 派生）；下载中则是重试 / 续传说明
    pub(crate) detail: String,
    /// 成功落盘的最终路径；未完成 / 失败为 None
    pub(crate) path: Option<PathBuf>,
    /// 5 秒速度窗口 + 卡住时钟（只在内存，见 `transfers_rate.rs`）
    pub(crate) speed: SpeedMeter,
    /// 下载中说明的存活状态（重试 / 续传说明恢复 5 秒后自动消失）
    pub(crate) note: NoteState,
    /// 文件夹总账行：一个文件都没有，建出目录就算完成（空文件夹）
    pub(crate) empty_folder: bool,
    /// 文件夹总账行：被跳过的目录数（过深 / 读失败 / 本地建目录失败）
    pub(crate) skipped: usize,
    /// 批次入队时刻（Unix 秒）：90 天清理与整批重试保留原时刻
    pub(crate) created_unix: i64,
    /// 进程启动序号：落盘后用来识别「上一轮没跑完的行」
    pub(crate) session: u64,
    /// 正常退出时仍在排队 / 准备 / 下载，下一轮恢复登录态后自动续跑
    pub(crate) auto_resume: bool,
    /// 上传阶段（只影响实时进度显示，不落盘）
    pub(crate) upload_progress: Option<UploadProgressState>,
    /// 上传文件夹父行：已创建的云端目录路径 → id，用于重试时复用而不重复建树。
    pub(crate) upload_dirs: Vec<UploadDir>,
    /// 上传文件夹父行：当前失败的云端目录路径；重试成功后逐项移除。
    ///
    /// 用路径而不是计数，才能在一棵子树重试成功后把父行从「失败」恢复，
    /// 同时保留其它仍未成功的子树。
    pub(crate) upload_failed_dirs: Vec<Vec<String>>,
    /// 文件夹总账行的子行聚合缓存；子行状态 / 字节变化时增量维护，
    /// 视图和父行状态机不再逐次扫描整批。
    pub(crate) batch_stats: BatchStats,
}

impl Transfer {
    pub(crate) fn display_name(&self) -> String {
        self.job.display_name()
    }
}
