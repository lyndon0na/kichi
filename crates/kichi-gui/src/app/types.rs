use std::path::PathBuf;
use std::time::Instant;

/// 传输任务页的上传/下载分栏。
#[derive(PartialEq, Eq, Clone, Copy)]
pub(crate) enum TransferTab {
    Upload,
    Download,
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

/// 离线任务页的阶段页签。
#[derive(PartialEq, Eq, Clone, Copy)]
pub(crate) enum OfflineTab {
    Pending,
    Running,
    Complete,
    Error,
}

impl OfflineTab {
    /// 展示顺序与服务端 phase 请求顺序一致。
    pub(crate) const ALL: [OfflineTab; 4] = [
        OfflineTab::Pending,
        OfflineTab::Running,
        OfflineTab::Complete,
        OfflineTab::Error,
    ];

    /// 对应的服务端 phase 常量。
    pub(crate) fn phase(self) -> &'static str {
        match self {
            OfflineTab::Pending => "PHASE_TYPE_PENDING",
            OfflineTab::Running => "PHASE_TYPE_RUNNING",
            OfflineTab::Complete => "PHASE_TYPE_COMPLETE",
            OfflineTab::Error => "PHASE_TYPE_ERROR",
        }
    }
}

/// 离线任务行级操作。
pub(crate) enum TaskOp {
    /// 下载到本地 (file_id, name)。
    Download(String, String),
    /// 重试失败任务 (task_id)。
    Retry(String),
    /// 删除任务记录 (task_id)。
    Delete(String),
}

/// 离线任务列表行点击产生的选择请求。
pub(crate) enum TaskSel {
    Replace(String),
    Toggle(String),
    Range(String),
}

/// 本地下载任务的 UI 状态。
#[derive(Clone, PartialEq, Debug)]
pub(crate) enum DlStatus {
    Queued,
    Running,
    Done,
    Failed(String),
}

/// 目录任务下的一个树节点(子目录或文件), 按先序存放。
#[derive(Clone)]
pub(crate) struct DlNode {
    pub is_dir: bool,
    pub name: String,
    /// 层级: 目录卡片(root)=0, 其直接子项=1。
    pub depth: u32,
    /// 文件节点对应的子任务 req_id; 目录节点为 None。
    pub rid: Option<u64>,
    /// 目录节点: 是否展开。
    pub expanded: bool,
    /// 目录节点: 子树内的文件完成数 / 总数。
    pub files_done: u32,
    pub files_total: u32,
    /// 文件节点: 是否已完成(用于目录节点的子树计数)。
    pub done: bool,
}

#[derive(Clone)]
pub(crate) struct DlJob {
    /// 云端文件 id, 用于失败/取消后重试(历史记录可能为空)。目录任务为目录 id。
    pub file_id: String,
    /// 对应的历史记录唯一标识, 用于精确移除(历史记录或首次写盘后填充)。
    pub record_id: String,
    pub name: String,
    pub dir: PathBuf,
    pub total: u64,
    pub done: u64,
    pub status: DlStatus,
    /// 估算速率(bytes/s)。
    pub speed: u64,
    pub last_done: u64,
    pub last_at: Option<Instant>,
    /// 完成/失败时间(unix 秒); 进行中为 None。
    pub at: Option<u64>,
    /// 目录任务: 云端目录 id; 普通文件/子文件为 None。
    pub folder_id: Option<String>,
    /// 子文件任务: 所属目录任务的 req_id; 顶层任务为 None。
    pub parent: Option<u64>,
    /// 目录任务: 目录树节点(先序, 含子目录与文件)。
    pub nodes: Vec<DlNode>,
    /// 目录任务: 是否展开整个目录树。
    pub expanded: bool,
    /// 目录任务: 已完成 / 全部文件数。
    pub files_done: u32,
    pub files_total: u32,
}

impl DlJob {
    pub fn queued(file_id: String, name: String, dir: PathBuf) -> Self {
        DlJob {
            file_id,
            record_id: String::new(),
            name,
            dir,
            total: 0,
            done: 0,
            status: DlStatus::Queued,
            speed: 0,
            last_done: 0,
            last_at: None,
            at: None,
            folder_id: None,
            parent: None,
            nodes: Vec::new(),
            expanded: false,
            files_done: 0,
            files_total: 0,
        }
    }

    /// 目录任务: 先以「扫描中」状态入队, 扫描完成后填充目录树。
    pub fn folder(folder_id: String, name: String, dir: PathBuf) -> Self {
        let mut job = DlJob::queued(folder_id.clone(), name, dir);
        job.folder_id = Some(folder_id);
        job
    }

    /// 子文件任务: 归属某个目录任务。
    pub fn child(file_id: String, name: String, dir: PathBuf, parent: u64) -> Self {
        let mut job = DlJob::queued(file_id, name, dir);
        job.parent = Some(parent);
        job
    }

    pub fn is_folder(&self) -> bool {
        self.folder_id.is_some()
    }

    /// 目录树内文件的子任务 req_id(按先序)。
    pub fn file_rids(&self) -> impl Iterator<Item = u64> + '_ {
        self.nodes.iter().filter_map(|n| n.rid)
    }
}

/// 下载窗口内的行级操作。
pub(crate) enum DlOp {
    Cancel,
    OpenDir,
    /// 用系统默认程序打开已下载的本地文件。
    OpenFile,
    /// 重新下载(仅本地会话内、已知云端 id 的任务可用)。
    Retry,
    Remove,
    /// 展开/收起整个目录任务的目录树。
    Expand,
    /// 展开/收起目录任务下的某个子目录节点(节点下标)。
    ToggleDir(usize),
}

/// 下载列表中的一个块: 单个任务卡片, 或一个展开目录(卡片内含目录树)。
pub(crate) enum DlRow {
    /// 普通任务卡片(或未展开的目录)。
    Job(u64),
    /// 展开的目录: 目录卡片 + 其可见的树节点下标(先序, 已跳过收起子目录的子孙)。
    Tree(u64, Vec<usize>),
}

/// 下载列表的状态筛选。
#[derive(PartialEq, Eq, Clone, Copy)]
pub(crate) enum DlFilter {
    All,
    Active,
    Done,
    Failed,
}

impl DlFilter {
    pub fn matches(&self, job: &DlJob) -> bool {
        match self {
            DlFilter::All => true,
            DlFilter::Active => matches!(job.status, DlStatus::Queued | DlStatus::Running),
            DlFilter::Done => job.status == DlStatus::Done,
            DlFilter::Failed => matches!(job.status, DlStatus::Failed(_)),
        }
    }
}

/// 本地上传任务的状态。
#[derive(Clone, PartialEq)]
pub(crate) enum UlStatus {
    Queued,
    Running,
    Done,
    Failed(String),
}

/// 本地上传任务 (上传到网盘某目录)。
#[derive(Clone)]
pub(crate) struct UlJob {
    pub local_path: PathBuf,
    pub name: String,
    /// 目标网盘目录 (None = 根目录)。
    pub parent: Option<String>,
    /// 目标目录的层级快照 (id, label), 用于展示与「在网盘中打开」。
    pub dest_stack: Vec<(Option<String>, String)>,
    pub total: u64,
    pub done: u64,
    pub status: UlStatus,
    /// 估算速率(bytes/s)。
    pub speed: u64,
    pub last_done: u64,
    pub last_at: Option<Instant>,
    /// 上传历史记录的唯一标识(历史记录或完成后填充)。
    pub record_id: String,
    /// 是否为目录递归上传。
    pub is_dir: bool,
    /// 目录上传的已完成/总文件数。
    pub files_done: u32,
    pub files_total: u32,
    /// 目录上传当前文件。
    pub current: String,
    /// 完成/失败时间(unix 秒); 进行中为 None。
    pub at: Option<u64>,
}

impl UlJob {
    /// 目标网盘路径展示, 如 "我的云盘 / 视频"。
    pub fn dest_label(&self) -> String {
        self.dest_stack
            .iter()
            .map(|(_, l)| l.clone())
            .collect::<Vec<_>>()
            .join(" / ")
    }

    pub fn queued(
        path: PathBuf,
        name: String,
        parent: Option<String>,
        dest_stack: Vec<(Option<String>, String)>,
    ) -> Self {
        UlJob {
            local_path: path,
            name,
            parent,
            dest_stack,
            total: 0,
            done: 0,
            status: UlStatus::Queued,
            speed: 0,
            last_done: 0,
            last_at: None,
            record_id: String::new(),
            is_dir: false,
            files_done: 0,
            files_total: 0,
            current: String::new(),
            at: None,
        }
    }

    pub fn queued_dir(
        path: PathBuf,
        name: String,
        parent: Option<String>,
        dest_stack: Vec<(Option<String>, String)>,
    ) -> Self {
        let mut job = Self::queued(path, name, parent, dest_stack);
        job.is_dir = true;
        job
    }
}

/// 上传任务行级操作。
pub(crate) enum UlOp {
    Cancel,
    Retry,
    Remove,
    /// 在「我的文件」中打开该上传的目标目录。
    OpenInDrive,
}

/// 上传列表的状态筛选。
#[derive(PartialEq, Eq, Clone, Copy)]
pub(crate) enum UlFilter {
    All,
    Active,
    Done,
    Failed,
}

impl UlFilter {
    pub fn matches(&self, job: &UlJob) -> bool {
        match self {
            UlFilter::All => true,
            UlFilter::Active => matches!(job.status, UlStatus::Queued | UlStatus::Running),
            UlFilter::Done => job.status == UlStatus::Done,
            UlFilter::Failed => matches!(job.status, UlStatus::Failed(_)),
        }
    }
}

/// 进行中的异步文件/目录选择: (是否目录, 目标目录 id, 目标路径快照, 结果通道)。
pub(crate) type UploadPick = (
    bool,
    Option<String>,
    Vec<(Option<String>, String)>,
    std::sync::mpsc::Receiver<Vec<PathBuf>>,
);

/// 已解析的可用清晰度与同集字幕。
pub(crate) struct QualityReady {
    pub options: Vec<crate::msg::QualityOption>,
    pub subs: Vec<PathBuf>,
}

/// 「播放」子菜单展示所需的清晰度状态。
#[derive(Clone, Copy)]
pub(crate) enum QualityMenuState<'a> {
    /// 尚未解析完成。
    Loading,
    /// 已解析(可能为空列表)。
    Ready(&'a QualityReady),
}

/// 待回传的「用系统程序打开」动作(后台探针结果)。
pub(crate) struct PendingOpen {
    pub rx: std::sync::mpsc::Receiver<super::helpers::OpenOutcome>,
    /// 提示里展示的目标名(文件名 / 目录路径)。
    pub label: String,
    /// 成功时不提示(打开目录保持安静, 只有失败才说话)。
    pub quiet_ok: bool,
}

/// 大文件预览确认弹窗的状态: 非媒体预览需先整份下载, 超过阈值时先问一次。
#[derive(Clone)]
pub(crate) struct PreviewConfirm {
    pub id: String,
    pub name: String,
    pub size: i64,
}

/// 非媒体预览的缓存下载进度, 驱动常驻进度条与取消按钮。
#[derive(Clone)]
pub(crate) struct PreviewProgress {
    pub req_id: u64,
    pub name: String,
    /// 总大小未知时为 0(进度条走不确定动画)。
    pub total: u64,
    pub done: u64,
}

/// 分享创建成功后的结果展示。
#[derive(Clone)]
pub(crate) struct ShareResult {
    pub url: String,
    pub pass_code: String,
    pub share_text: String,
    pub label: String,
}
