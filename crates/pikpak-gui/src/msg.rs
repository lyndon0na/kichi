use pikpak_core::types::{FileList, Quota, Task};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// UI -> 后台线程 指令。
pub enum Cmd {
    /// 账号密码登录。
    Login {
        username: String,
        password: String,
    },
    /// 用磁盘上的会话恢复登录。
    Resume {
        device_id: String,
        access_token: String,
        refresh_token: String,
        user_id: String,
        username: String,
    },
    Logout,
    /// 列出某目录文件。append=true 表示加载下一页。
    ListFiles {
        parent: Option<String>,
        token: Option<String>,
        append: bool,
        req_id: u64,
    },
    CreateFolder {
        name: String,
        parent: Option<String>,
    },
    Rename {
        id: String,
        name: String,
    },
    Trash {
        ids: Vec<String>,
    },
    /// 移动/复制目标目录选择: 列出某目录的子项(取其中的文件夹)。append=true 表示下一页。
    ListFolders {
        parent: Option<String>,
        token: Option<String>,
        append: bool,
        req_id: u64,
    },
    /// 批量移动选中的文件到目标目录(None = 我的云盘根目录)。
    MoveTo {
        ids: Vec<String>,
        dest: Option<String>,
    },
    /// 批量复制选中的文件到目标目录。
    CopyTo {
        ids: Vec<String>,
        dest: Option<String>,
    },
    OfflineCreate {
        url: String,
        name: Option<String>,
        parent: Option<String>,
    },
    RefreshTasks,
    OfflineRetry {
        task_id: String,
    },
    OfflineDelete {
        task_ids: Vec<String>,
        delete_files: bool,
    },
    RefreshQuota,
    /// 把云端文件(file_id)下载到本地目录 dest_dir。
    /// 文件最终名由后台取详情后用 `name` 兜底, 再自动去重(必要时追加 " (n)")。
    StartDownload {
        req_id: u64,
        file_id: String,
        name: String,
        dest_dir: PathBuf,
    },
    /// 取消某个下载任务(保留已下载的 .part, 便于下次续传)。
    CancelDownload {
        req_id: u64,
    },
}

/// 后台线程 -> UI 消息。
pub enum Msg {
    LoginOk {
        username: String,
    },
    LoggedOut,
    SessionInvalid {
        reason: String,
    },
    Files {
        parent: Option<String>,
        req_id: u64,
        append: bool,
        list: FileList,
    },
    FolderCreated,
    Renamed,
    Trashed,
    /// 移动/复制目标目录选择器的子文件夹列表。
    FoldersList {
        req_id: u64,
        parent: Option<String>,
        append: bool,
        list: FileList,
    },
    Moved {
        ids: Vec<String>,
    },
    Copied,
    OfflineCreated,
    OfflineRetried,
    OfflineDeleted,
    Quota(Option<Quota>),
    TasksAll {
        buckets: BTreeMap<String, Vec<Task>>,
    },
    /// 下载进度。total 未知时为 0。
    DlProgress {
        req_id: u64,
        total: u64,
        done: u64,
    },
    /// 下载完成。bytes 为写入字节数。
    DlFinished {
        req_id: u64,
        bytes: u64,
    },
    /// 下载被取消。
    DlCancelled {
        req_id: u64,
    },
    /// 下载失败。
    DlFailed {
        req_id: u64,
        what: String,
    },
    Error {
        what: String,
    },
}
