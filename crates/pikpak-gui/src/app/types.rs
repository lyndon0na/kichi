use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Instant;

#[derive(PartialEq, Clone, Copy)]
pub(crate) enum Page {
    Files,
    Tasks,
    Downloads,
    Settings,
}

#[derive(PartialEq, Eq, Clone, Copy, PartialOrd, Ord, Hash)]
pub(crate) enum SortBy {
    Name,
    Size,
    Modified,
}

#[derive(Clone)]
pub(crate) struct Crumb {
    pub id: Option<String>,
    pub label: String,
}

#[derive(Clone)]
pub(crate) enum RowAction {
    OpenFolder(String, String),
    OpenFile,
    DownloadFile(String, String),
    CopyName(String),
    Rename(String, String),
    MoveToFolder(String),
    CopyToFolder(String),
    Trash(String),
}

/// 行点击产生的选择请求。
pub(crate) enum RowSel {
    /// 普通单击: 只选中该项(替换原选择)。
    Replace(String),
    /// Ctrl+单击: 在选中/未选中之间切换。
    Toggle(String),
    /// Shift+单击: 范围选择(从上次点击项到当前项)。
    Range(String),
}

/// 下载列表行点击产生的选择请求。
pub(crate) enum DlSel {
    /// 普通单击: 只选中该项(替换原选择)。
    Replace(u64),
    /// Ctrl+单击: 在选中/未选中之间切换。
    Toggle(u64),
    /// Shift+单击: 范围选择。
    Range(u64),
}

/// 文件列表视图模式。
#[derive(PartialEq, Eq, Clone, Copy)]
pub(crate) enum ViewMode {
    /// 列表视图(表格样式)。
    List,
    /// 图标视图(网格缩略图)。
    Icon,
}

/// 移动/复制操作的类型。
#[derive(PartialEq, Eq, Clone, Copy)]
pub(crate) enum MoveMode {
    Move,
    Copy,
}

impl MoveMode {
    pub fn title(self) -> &'static str {
        match self {
            MoveMode::Move => "移动文件到…",
            MoveMode::Copy => "复制文件到…",
        }
    }
    pub fn verb(self) -> &'static str {
        match self {
            MoveMode::Move => "移动",
            MoveMode::Copy => "复制",
        }
    }
}

/// 「移动/复制到…」的目标目录选择弹窗状态。
pub(crate) struct MoveDialog {
    pub mode: MoveMode,
    /// 本次要操作的源文件 id。
    pub ids: Vec<String>,
    /// 源所在目录(目标等于它时表示未移动, 用于禁用确认按钮)。
    pub src_parent: Option<String>,
    /// 不可进入的目录(即被选中的源文件夹), 避免把文件夹移/复制进它自己。
    pub blocked: HashSet<String>,
    /// 当前浏览路径, 栈顶即目标目录。
    pub stack: Vec<Crumb>,
    /// 当前目录下的子文件夹(已剔除自身), 展示在列表中。
    pub folders: Vec<pikpak_core::types::File>,
    pub next: Option<String>,
    pub loading: bool,
    pub req_id: u64,
}

/// 本地下载任务的 UI 状态。
#[derive(Clone, PartialEq)]
pub(crate) enum DlStatus {
    Queued,
    Running,
    Done,
    Cancelled,
    Failed(String),
}

#[derive(Clone)]
pub(crate) struct DlJob {
    pub name: String,
    pub dir: PathBuf,
    pub total: u64,
    pub done: u64,
    pub status: DlStatus,
    /// 估算速率(bytes/s)。
    pub speed: u64,
    pub last_done: u64,
    pub last_at: Option<Instant>,
}

impl DlJob {
    pub fn queued(name: String, dir: PathBuf) -> Self {
        DlJob {
            name,
            dir,
            total: 0,
            done: 0,
            status: DlStatus::Queued,
            speed: 0,
            last_done: 0,
            last_at: None,
        }
    }
}

/// 下载窗口内的行级操作。
pub(crate) enum DlOp {
    Cancel,
    OpenDir,
    Remove,
}

/// 列拖拽状态。
pub(crate) struct ColDrag {
    /// 拖拽的是哪条分割线 (1 = 名称|大小, 2 = 大小|修改时间)
    pub handle: u8,
    /// 拖拽开始时鼠标 x
    pub start_x: f32,
    /// 拖拽开始时 col_size_w
    pub orig_size_w: f32,
    /// 拖拽开始时 col_time_w
    pub orig_time_w: f32,
}
