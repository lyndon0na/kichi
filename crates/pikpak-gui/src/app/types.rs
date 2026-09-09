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
