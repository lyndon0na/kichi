use pikpak_core::types::{FileList, Quota, Task};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// 一个可选择的清晰度: 标签、限时直链, 以及播放时服务端接受的请求头。
#[derive(Clone)]
pub struct QualityOption {
    pub label: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
}

/// UI -> 后台线程 指令。
pub enum Cmd {
    /// 账号密码登录。
    Login {
        username: String,
        password: String,
    },
    /// 用系统密钥环里保存的密码自动登录。
    AutoLogin {
        username: String,
    },
    /// 把账号密码写入系统密钥环(记住密码)。
    RememberPassword {
        username: String,
        password: String,
    },
    /// 从系统密钥环删除已保存的密码。
    ForgetPassword {
        username: String,
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
    /// 批量移动选中的文件到目标目录(None = 我的云盘根目录)。
    /// src 为源目录, 用于移动成功后在源目录本地隐藏(区分目标目录)。
    MoveTo {
        ids: Vec<String>,
        dest: Option<String>,
        src: Option<String>,
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
    /// 列出某目录下的子文件夹(用于离线下载「保存到」网盘目录选择)。
    ListFolders {
        parent: Option<String>,
        req_id: u64,
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
    /// 把本地文件上传到网盘目录 parent(None = 根目录)。
    StartUpload {
        req_id: u64,
        path: PathBuf,
        parent: Option<String>,
    },
    /// 把本地目录递归上传到网盘目录 parent(None = 根目录)。
    StartUploadDir {
        req_id: u64,
        path: PathBuf,
        parent: Option<String>,
    },
    /// 取消某个上传任务。
    CancelUpload {
        req_id: u64,
    },
    /// 预览云端文件。media=true 时仅解析直链交给外部播放器(mpv)流式播放;
    /// media=false 时先下载到本地缓存再交给系统查看器打开。
    /// `subtitles` 为同集字幕 (file_id, 文件名), 仅媒体预览使用。
    Preview {
        req_id: u64,
        file_id: String,
        name: String,
        media: bool,
        subtitles: Vec<(String, String)>,
    },
    /// 解析媒体文件可用清晰度, 供预览时选择画质。
    PreviewQualities {
        file_id: String,
        subtitles: Vec<(String, String)>,
    },
}

/// 后台线程 -> UI 消息。
pub enum Msg {
    LoginOk {
        username: String,
    },
    /// 登录失败(手动或密钥环自动登录)。
    LoginFailed {
        what: String,
    },
    /// 自动登录不可用(密钥环未保存密码或不可用), 应回落到登录表单。
    AutoLoginUnavailable,
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
    /// 列出目录失败; 带上 parent 以便精确结束加载态并释放在途登记。
    FilesFailed {
        parent: Option<String>,
        what: String,
    },
    FolderCreated,
    Renamed,
    Trashed,
    Moved {
        ids: Vec<String>,
        src: Option<String>,
        dest: Option<String>,
    },
    Copied {
        dest: Option<String>,
    },
    OfflineCreated,
    OfflineRetried,
    OfflineDeleted,
    /// 离线下载目录选择器所需的子文件夹列表。
    Folders {
        parent: Option<String>,
        req_id: u64,
        files: Vec<pikpak_core::types::File>,
    },
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
    /// 上传进度。total 未知时为 0。
    UlProgress {
        req_id: u64,
        total: u64,
        done: u64,
    },
    /// 上传完成。
    UlFinished {
        req_id: u64,
    },
    /// 目录上传的子文件计数/当前文件。
    UlFiles {
        req_id: u64,
        done: u32,
        total: u32,
        current: String,
    },
    /// 上传被取消。
    UlCancelled {
        req_id: u64,
    },
    /// 上传失败。
    UlFailed {
        req_id: u64,
        what: String,
    },
    /// 媒体预览直链已解析, 可交给外部播放器(mpv)流式播放。
    PreviewStream {
        req_id: u64,
        name: String,
        url: String,
        /// 访问直链所需的请求头(User-Agent / X-Device-Id / 可选 Bearer)。
        headers: Vec<(String, String)>,
        /// 已下载到本地的同集外挂字幕路径。
        subs: Vec<PathBuf>,
    },
    /// 非媒体文件已下载到本地缓存, 可交给系统查看器打开。
    PreviewReady {
        req_id: u64,
        name: String,
        path: PathBuf,
    },
    /// 媒体文件可用清晰度列表已解析。原画在前, 每项自带解析好的请求头。
    PreviewQualities {
        file_id: String,
        qualities: Vec<QualityOption>,
        /// 已下载到本地的同集外挂字幕路径。
        subs: Vec<PathBuf>,
    },
    /// 清晰度解析失败(供「播放」子菜单结束加载态)。
    QualitiesFailed {
        file_id: String,
        what: String,
    },
    /// 预览准备失败(解析直链或下载出错)。
    PreviewFailed {
        req_id: u64,
        what: String,
    },
    Error {
        what: String,
    },
}
