use eframe::egui;
use kichi_core::types::{FileList, Quota, ShareList, Task};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// 目录扫描出的一个条目(目录或文件), 按先序排列并带层级。
#[derive(Clone)]
pub struct FolderItem {
    pub is_dir: bool,
    pub name: String,
    /// 层级: 目录卡片(root)=0, 其直接子项=1。
    pub depth: u32,
    /// 文件条目的云端 id(目录条目为空)。
    pub file_id: String,
    /// 文件条目: 目标目录; 目录条目: 该目录的本地路径。
    pub dir: PathBuf,
}

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
    /// 全局搜索文件(跨目录)。append=true 表示加载下一页。
    SearchFiles {
        keyword: String,
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
    /// 列出回收站内容。append=true 表示加载下一页。
    ListTrash {
        token: Option<String>,
        append: bool,
        req_id: u64,
    },
    /// 从回收站还原选中项。
    Untrash {
        ids: Vec<String>,
    },
    /// 从回收站彻底删除选中项(不可恢复)。
    DeleteTrash {
        ids: Vec<String>,
    },
    /// 清空整个回收站(服务端一次完成)。
    EmptyTrash,
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
    /// 加载某个 phase 的下一页离线任务。
    LoadMoreTasks {
        phase: String,
    },
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
    /// 递归下载云端目录(folder_id)到本地目录 dest_dir。
    /// 后台先扫描目录树, 回 `Msg::FolderScanned`, 再由 UI 逐个提交子文件下载。
    StartDownloadFolder {
        req_id: u64,
        folder_id: String,
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
    /// 为选中项创建分享链接。expiration_days=-1 表示永久。
    CreateShare {
        file_ids: Vec<String>,
        expiration_days: i64,
        need_password: bool,
        /// 单个文件时为其名称, 多项时形如 "3 项", 用于结果展示。
        label: String,
    },
    /// 列出「我的分享」。append=true 表示加载下一页。
    ListShares {
        token: Option<String>,
        append: bool,
        req_id: u64,
    },
    /// 批量取消分享。
    DeleteShares {
        ids: Vec<String>,
    },
    /// 解析他人分享链接, 获取文件列表。
    ResolveShare {
        share_id: String,
        pass_code: String,
    },
    /// 加载更多分享文件(分页)。
    LoadMoreShareFiles {
        share_id: String,
        pass_code_token: String,
        page_token: String,
    },
    /// 将分享中的文件转存到自己的网盘。dest 为转存后自动移动到的目标目录。
    SaveShare {
        share_id: String,
        pass_code_token: String,
        file_ids: Vec<String>,
        dest: Option<String>,
    },
    /// 重试移动转存文件到目标目录(自动移动失败后手动重试)。
    RetryMoveShare {
        dest: String,
    },
    /// 加载缩略图: 从 URL 下载到本地缓存, 解码后发送 ThumbnailReady。
    LoadThumbnail {
        file_id: String,
        url: String,
    },
    /// 更新传输并发/重试参数。并发即时生效(不打断在传任务),
    /// 重试与分片并发对之后新启动的任务生效。
    SetTransferLimits {
        dl_concurrency: usize,
        ul_concurrency: usize,
        part_concurrency: usize,
        max_attempts: usize,
    },
    /// 维护磁盘缓存(预览 / 缩略图)。`purge=true` 清空, 否则只按上限淘汰。
    /// 处理完回传 `Msg::CacheUsage`, 供设置页展示占用。
    MaintainCache {
        purge: bool,
    },
}

/// 后台线程 -> UI 消息。
pub enum Msg {
    LoginOk {
        username: String,
    },
    /// 登录失败(手动或密钥环自动登录)。
    /// `verify_url` 非空时表示服务端要求人机验证, 可在浏览器打开该链接完成后重试。
    LoginFailed {
        what: String,
        verify_url: Option<String>,
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
    /// 全局搜索结果。
    SearchResults {
        req_id: u64,
        append: bool,
        list: FileList,
    },
    /// 搜索失败。
    SearchFailed {
        what: String,
    },
    FolderCreated,
    Renamed,
    Trashed,
    /// 回收站列表。
    TrashList {
        req_id: u64,
        append: bool,
        list: FileList,
    },
    /// 加载回收站失败。
    TrashFailed {
        what: String,
    },
    /// 回收站还原成功(带 id 以便即时移除)。
    TrashRestored {
        ids: Vec<String>,
    },
    /// 回收站彻底删除成功(带 id 以便即时移除)。
    TrashDeleted {
        ids: Vec<String>,
    },
    /// 回收站已清空。
    TrashEmptied,
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
        files: Vec<kichi_core::types::File>,
    },
    Quota(Option<Quota>),
    TasksAll {
        buckets: BTreeMap<String, Vec<Task>>,
        next_tokens: BTreeMap<String, Option<String>>,
    },
    /// 某 phase 追加了一页离线任务(append 语义)。
    TasksMore {
        phase: String,
        tasks: Vec<Task>,
        next_page_token: Option<String>,
    },
    TasksMoreFailed {
        phase: String,
        what: String,
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
    /// 目录扫描完成: items 为先序排列的目录树条目, total_bytes 为文件合计大小。
    FolderScanned {
        req_id: u64,
        items: Vec<FolderItem>,
        total_bytes: u64,
    },
    /// 目录扫描失败。
    FolderScanFailed {
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
    /// 分享创建成功。
    ShareCreated {
        url: String,
        pass_code: String,
        share_text: String,
        label: String,
    },
    /// 「我的分享」列表。
    Shares {
        req_id: u64,
        append: bool,
        list: ShareList,
    },
    /// 加载「我的分享」失败。
    SharesFailed {
        what: String,
    },
    /// 分享已取消(带上 id 以便即时移除)。
    SharesDeleted {
        ids: Vec<String>,
    },
    /// 他人分享链接已解析, 包含文件列表。
    ShareResolved {
        share_id: String,
        title: String,
        pass_code_token: String,
        files: Vec<kichi_core::types::File>,
        next_page_token: Option<String>,
    },
    /// 加载更多分享文件完成。
    ShareFilesLoaded {
        files: Vec<kichi_core::types::File>,
        next_page_token: Option<String>,
    },
    /// 加载更多分享文件失败。
    ShareFilesLoadFailed {
        what: String,
    },
    /// 解析分享链接失败。
    ShareResolveFailed {
        what: String,
    },
    /// 转存成功。auto_move_failed 表示转存成功但自动移动到目标目录失败。
    ShareSaved {
        auto_move_failed: bool,
    },
    /// 转存失败。
    ShareSaveFailed {
        what: String,
    },
    /// 重试移动成功。
    ShareMoveRetried,
    /// 重试移动失败。
    ShareMoveRetryFailed {
        what: String,
    },
    /// 缩略图已加载并解码为 RGBA 像素。
    ThumbnailReady {
        file_id: String,
        width: u32,
        height: u32,
        pixels: Vec<egui::Color32>,
    },
    /// 磁盘缓存占用(预览 + 缩略图)。`freed` 为本次操作释放的字节数。
    CacheUsage {
        bytes: u64,
        entries: usize,
        freed: u64,
    },
    Error {
        what: String,
    },
}
