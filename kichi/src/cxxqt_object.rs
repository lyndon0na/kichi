//! 桥接对象：Rust 侧暴露给 QML 的登录状态与控制入口。
//!
//! cxx-qt 0.10 踩坑记录（均已实测复现）：
//! 1. 桥接里**不要**声明 `impl cxx_qt::Initialize for Kichi {}`。
//!    cxx-qt 只在「没有任何构造器声明」时自动生成默认构造器
//!    （内部调用 `KichiRust::default()`）；一旦写了这个标记，
//!    就要求你在桥接外真正实现 `Initialize`，否则编译报
//!    `the trait bound Kichi: Initialize is not satisfied`。
//! 2. 桥接上**不要**用 `#[cxx_qt::bridge(namespace = "...")]`：
//!    它会把 `QString` 等 C++ 类型也塞进该命名空间，生成代码报
//!    `'kichi::QString' has not been declared`。QML 类型名由
//!    `QML_NAMED_ELEMENT` 决定，不需要 C++ 命名空间。
//! 3. 属性必须带 NOTIFY 信号，否则 QML 永远不会重新求值绑定
//!    （现象：点登录毫无反应、状态文字不更新）。这里所有属性共用一个
//!    `changed` 信号，Rust 侧每次改完字段调用 `notify()` 即可。
//! 4. 桥接里不要出现 `impl cxx_qt::Threading`，线程回投会失效；
//!    后台结果走主线程队列（QML 定时器调用 `pump` 抽水），见 `actions.rs`。
//! 5. invokable 声明端与实现端签名要一致：`Pin<&mut Self>` 对
//!    `Pin<&mut Self>`，或 `&mut Self` 对 `&mut Self`。

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;

        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;

        include!("cxx-qt-lib/qlist.h");
        include!("cxx-qt-lib/qvariant.h");
        type QVariantList = crate::cxxqt_object::QVariantList;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QString, auth_state, cxx_name = "authState", READ, NOTIFY = changed)]
        #[qproperty(QString, status_message, cxx_name = "statusMessage", READ, NOTIFY = changed)]
        /// 登录请求在途：只给登录页用，后台任务不得修改
        #[qproperty(bool, login_running, cxx_name = "loginRunning", READ, NOTIFY = changed)]
        #[qproperty(QString, account, READ, NOTIFY = changed)]
        /// `/v1/user/me` 的昵称；空串时 QML 从账号里取用户名兜底
        #[qproperty(QString, display_name, cxx_name = "displayName", READ, NOTIFY = changed)]
        /// 已下载到本机的头像 `file://` URI；空串时 QML 画昵称首字母
        #[qproperty(QString, avatar_path, cxx_name = "avatarPath", READ, NOTIFY = changed)]
        #[qproperty(QString, quota_used_text, cxx_name = "quotaUsed", READ, NOTIFY = changed)]
        #[qproperty(QString, quota_limit_text, cxx_name = "quotaLimit", READ, NOTIFY = changed)]
        /// 容量请求在途；账号身份和容量分别加载，避免昵称缓存让两者一起“完成”
        #[qproperty(bool, quota_loading, cxx_name = "quotaLoading", READ, NOTIFY = changed)]
        /// 容量占用比例 0..1；`-1` = 不限量 / 未知，QML 据此隐藏进度条
        #[qproperty(f64, quota_ratio, cxx_name = "quotaRatio", READ, NOTIFY = changed)]
        #[qproperty(bool, quota_vip, cxx_name = "quotaVip", READ, NOTIFY = changed)]
        #[qproperty(bool, captcha_required, cxx_name = "captchaRequired", READ, NOTIFY = changed)]
        #[qproperty(QVariantList, files, READ, NOTIFY = changed)]
        /// 当前目录 id（根为空串）：界面按它记忆 / 恢复滚动位置
        #[qproperty(QString, current_dir_id, cxx_name = "currentDirId", READ, NOTIFY = changed)]
        /// 首次读取或用户手动刷新：文件页自己的可见加载态
        #[qproperty(bool, listing, cxx_name = "filesLoading", READ, NOTIFY = changed)]
        /// 用户手动刷新已有目录；TTL 自动校准不会置位
        #[qproperty(bool, files_manual_refreshing, cxx_name = "filesRefreshing", READ, NOTIFY = changed)]
        /// 正在从离线任务打开 / 定位的产物 id；空串表示没有这项操作
        #[qproperty(QString, task_folder_loading_id, cxx_name = "taskFolderLoadingId", READ, NOTIFY = changed)]
        /// 每次成功读取目录 +1；界面据此做一次性的滚动恢复
        #[qproperty(i32, files_revision, cxx_name = "filesRevision", READ, NOTIFY = changed)]
        #[qproperty(QStringList, breadcrumb, READ, NOTIFY = changed)]
        /// 回收站表格（QML 直接当 model 用）。
        /// 不能叫 `trashEntries`：这个名字已被下面的移入回收站 invokable 占用。
        #[qproperty(QVariantList, trash_rows, cxx_name = "trashFiles", READ, NOTIFY = changed)]
        /// 首次读取或用户手动刷新：TTL 自动校准不会置位
        #[qproperty(bool, trash_loading, cxx_name = "trashLoading", READ, NOTIFY = changed)]
        #[qproperty(bool, trash_loaded, cxx_name = "trashLoaded", READ, NOTIFY = changed)]
        #[qproperty(QString, trash_error, cxx_name = "trashError", READ, NOTIFY = changed)]
        /// 回收站列表真正替换时 +1；选择清理与滚动位置恢复据此触发
        #[qproperty(i32, trash_revision, cxx_name = "trashRevision", READ, NOTIFY = changed)]
        /// 有云端条目从回收站还原，下次显示文件页时需要重新拉取
        #[qproperty(bool, files_stale, cxx_name = "filesStale", READ, NOTIFY = changed)]
        #[qproperty(QVariantList, tasks, READ, NOTIFY = changed)]
        /// 首次读取或用户手动刷新在途：QML 据此显示可见加载态
        #[qproperty(bool, tasks_loading, cxx_name = "tasksLoading", READ, NOTIFY = changed)]
        /// 正在逐条清理已结束任务：阻止重复入口并显示按钮忙碌态
        #[qproperty(bool, tasks_clearing, cxx_name = "tasksClearing", READ, NOTIFY = changed)]
        #[qproperty(QVariantList, shares, READ, NOTIFY = changed)]
        /// 首次读取或用户手动刷新：TTL 自动校准不会置位
        #[qproperty(bool, shares_loading, cxx_name = "sharesLoading", READ, NOTIFY = changed)]
        /// 资料库三页共用的当前集合；`libraryKind` 标明这些行属于哪一页。
        #[qproperty(QVariantList, library_rows, cxx_name = "libraryRows", READ, NOTIFY = changed)]
        #[qproperty(QString, library_kind, cxx_name = "libraryKind", READ, NOTIFY = changed)]
        /// 首次读取或用户手动刷新：TTL 自动校准不会置位
        #[qproperty(bool, library_loading, cxx_name = "libraryLoading", READ, NOTIFY = changed)]
        #[qproperty(bool, library_loaded, cxx_name = "libraryLoaded", READ, NOTIFY = changed)]
        #[qproperty(QString, library_error, cxx_name = "libraryError", READ, NOTIFY = changed)]
        /// 集合真正替换时 +1；QML 据此把当前行复位并恢复焦点。
        #[qproperty(i32, library_revision, cxx_name = "libraryRevision", READ, NOTIFY = changed)]
        /// 当前资料库集合的数据版本；仅条目真正变化时 +1，切页恢复缓存不推进。
        #[qproperty(i32, library_data_revision, cxx_name = "libraryDataRevision", READ, NOTIFY = changed)]
        /// 有创建或撤销分享的写请求在途
        #[qproperty(bool, share_action_running, cxx_name = "shareActionRunning", READ, NOTIFY = changed)]
        /// 刚创建成功的分享结果（QML 弹窗切到成功态后读取）
        #[qproperty(QString, share_created_url, cxx_name = "shareCreatedUrl", READ, NOTIFY = changed)]
        #[qproperty(QString, share_created_pass_code, cxx_name = "shareCreatedPassCode", READ, NOTIFY = changed)]
        #[qproperty(QString, share_created_expiration, cxx_name = "shareCreatedExpiration", READ, NOTIFY = changed)]
        #[qproperty(QVariantList, play_menu, cxx_name = "playMenu", READ, NOTIFY = changed)]
        #[qproperty(QString, play_title, cxx_name = "playTitle", READ, NOTIFY = changed)]
        /// 取链或等播放器就绪：文件缩略图据此显示加载状态
        #[qproperty(bool, play_loading, cxx_name = "playLoading", READ, NOTIFY = changed)]
        /// 正在加载的文件 id（缩略图按 id 对位，不能按文件名猜）
        #[qproperty(QString, play_loading_id, cxx_name = "playLoadingId", READ, NOTIFY = changed)]
        /// 正在加载的文件名
        #[qproperty(QString, play_loading_name, cxx_name = "playLoadingName", READ, NOTIFY = changed)]
        /// 播放器已经真正开播、尚未退出的文件 id（可同时有多个）
        #[qproperty(QStringList, playing_ids, cxx_name = "playingIds", READ, NOTIFY = changed)]
        /// 非空 = 提示条显示错误态（播放失败），[关闭] 清空
        #[qproperty(QString, play_error, cxx_name = "playError", READ, NOTIFY = changed)]
        /// 非空 = 提示条短提示（档位回退说明），几秒后由界面自动收起
        #[qproperty(QString, play_notice, cxx_name = "playNotice", READ, NOTIFY = changed)]
        #[qproperty(QString, preferred_quality, cxx_name = "preferredQuality", READ, NOTIFY = changed)]
        /// 播放器预设："mpv" / "vlc" / "custom"
        #[qproperty(QString, player_backend, cxx_name = "playerBackend", READ, NOTIFY = changed)]
        #[qproperty(QString, player_command, cxx_name = "playerCommand", READ, NOTIFY = changed)]
        #[qproperty(QString, player_args, cxx_name = "playerArgs", READ, NOTIFY = changed)]
        #[qproperty(QString, player_default_command, cxx_name = "playerDefaultCommand", READ, NOTIFY = changed)]
        #[qproperty(QString, player_default_args, cxx_name = "playerDefaultArgs", READ, NOTIFY = changed)]
        #[qproperty(bool, notify_enabled, cxx_name = "notifyEnabled", READ, NOTIFY = changed)]
        /// 播放时自动加载同目录字幕（只对 mpv 生效）
        #[qproperty(bool, auto_subtitles, cxx_name = "autoSubtitles", READ, NOTIFY = changed)]
        #[qproperty(bool, close_to_tray, cxx_name = "closeToTray", READ, NOTIFY = changed)]
        #[qproperty(bool, tray_available, cxx_name = "trayAvailable", READ, NOTIFY = changed)]
        #[qproperty(i32, tray_show_requested, cxx_name = "trayShowRequested", READ, NOTIFY = changed)]
        #[qproperty(bool, quit_requested, cxx_name = "quitRequested", READ, NOTIFY = changed)]
        /// 文件类内容页共用显示方式："list" / "grid"
        #[qproperty(QString, content_view_text, cxx_name = "contentView", READ, NOTIFY = changed)]
        #[qproperty(QString, sort_field_text, cxx_name = "sortField", READ, NOTIFY = changed)]
        #[qproperty(bool, sort_ascending, cxx_name = "sortAscending", READ, NOTIFY = changed)]
        #[qproperty(i32, window_width, cxx_name = "windowWidth", READ, NOTIFY = changed)]
        #[qproperty(i32, window_height, cxx_name = "windowHeight", READ, NOTIFY = changed)]
        #[qproperty(bool, window_maximized, cxx_name = "windowMaximized", READ, NOTIFY = changed)]
        #[qproperty(QString, settings_path, cxx_name = "settingsPath", READ, NOTIFY = changed)]
        #[qproperty(QString, storage_backend, cxx_name = "storageBackend", READ, NOTIFY = changed)]
        #[qproperty(bool, download_running, cxx_name = "downloadRunning", READ, NOTIFY = changed)]
        /// 排队中的批次数（不含正在跑的那一批）
        #[qproperty(i32, download_queued, cxx_name = "downloadQueued", READ, NOTIFY = changed)]
        /// 排队中的上传文件数（上传串行，不含正在上传的那一个）
        #[qproperty(i32, upload_queued, cxx_name = "uploadQueued", READ, NOTIFY = changed)]
        /// 正在上传的文件数（串行队列，只会是 0 或 1）
        #[qproperty(i32, upload_active, cxx_name = "uploadActive", READ, NOTIFY = changed)]
        #[qproperty(QVariantList, transfers, READ, NOTIFY = changed)]
        /// 传输记录的结构版本号：只有排队 / 开始 / 完成 / 失败 / 取消 / 清空才 +1。
        /// QML 用它判断该不该重新读取 `transfers`，避免进度节拍把整个
        /// ListView 模型重灌（会把滚动位置冲回顶部）。
        #[qproperty(i32, transfer_revision, cxx_name = "transferRevision", READ, NOTIFY = changed)]
        #[qproperty(QVariantList, transfer_live, cxx_name = "transferLive", READ, NOTIFY = changed)]
        /// 当前展开的文件夹批次的子文件列表（内联子行专用）
        #[qproperty(QVariantList, transfer_detail, cxx_name = "transferDetail", READ, NOTIFY = changed)]
        /// 文件夹子行快照是否有效（false 时父行不可展开）
        #[qproperty(bool, transfer_detail_active, cxx_name = "transferDetailActive", READ, NOTIFY = changed)]
        #[qproperty(i32, download_active, cxx_name = "downloadActive", READ, NOTIFY = changed)]
        /// 全部历史里有没有「所有行都已结束」的批次（清空按钮据此置灰）
        #[qproperty(bool, transfer_has_finished, cxx_name = "transferHasFinished", READ, NOTIFY = changed)]
        /// 是否存在可批量重试的失败记录（页头按钮据此显示）
        #[qproperty(bool, transfer_has_retryable, cxx_name = "transferHasRetryable", READ, NOTIFY = changed)]
        #[qproperty(QString, download_dir, cxx_name = "downloadDir", READ, NOTIFY = changed)]
        #[qproperty(i32, download_parallel, cxx_name = "downloadParallel", READ, NOTIFY = changed)]
        #[qproperty(i32, download_connections, cxx_name = "downloadConnections", READ, NOTIFY = changed)]
        #[qproperty(i32, download_rate_mib, cxx_name = "downloadRateMib", READ, NOTIFY = changed)]
        #[qproperty(bool, auto_resume_downloads, cxx_name = "autoResumeDownloads", READ, NOTIFY = changed)]
        #[qproperty(bool, file_action_running, cxx_name = "fileActionRunning", READ, NOTIFY = changed)]
        /// 移动 / 复制选择器当前层的文件夹（QML 当 model 用）
        #[qproperty(QVariantList, move_targets, cxx_name = "moveTargets", READ, NOTIFY = changed)]
        #[qproperty(bool, move_targets_loading, cxx_name = "moveTargetsLoading", READ, NOTIFY = changed)]
        #[qproperty(QString, move_targets_error, cxx_name = "moveTargetsError", READ, NOTIFY = changed)]
        /// 文件夹列表真正被替换时才 +1：无关的 `changed` 不该重置选择器滚动位置
        #[qproperty(i32, move_targets_revision, cxx_name = "moveTargetsRevision", READ, NOTIFY = changed)]
        #[qproperty(bool, search_active, cxx_name = "searchActive", READ, NOTIFY = changed)]
        #[qproperty(QString, search_query, cxx_name = "searchQuery", READ, NOTIFY = changed)]
        #[qproperty(QVariantList, search_results, cxx_name = "searchResults", READ, NOTIFY = changed)]
        #[qproperty(i32, search_scanned, cxx_name = "searchScanned", READ, NOTIFY = changed)]
        #[qproperty(i32, search_hits, cxx_name = "searchHits", READ, NOTIFY = changed)]
        #[qproperty(bool, search_settled, cxx_name = "searchSettled", READ, NOTIFY = changed)]
        #[qproperty(QString, search_status, cxx_name = "searchStatus", READ, NOTIFY = changed)]
        #[qproperty(bool, search_truncated, cxx_name = "searchTruncated", READ, NOTIFY = changed)]
        type Kichi = super::KichiRust;

        /// 统一变更通知：所有属性都挂这一个信号
        #[qsignal]
        fn changed(self: Pin<&mut Self>);

        /// 缩略图已落到本地缓存（`path` 是可直接给 Image 的 file:// URI）。
        /// 失败时 `path` 为空串，QML 退回文件图标。
        #[qsignal]
        #[cxx_name = "thumbnailReady"]
        fn thumbnail_ready(self: Pin<&mut Self>, id: QString, path: QString);

        /// 一次文件操作（改名 / 新建 / 删除）结束：动作、成功与否、给用户看的提示
        #[qsignal]
        #[cxx_name = "fileActionDone"]
        fn file_action_done(self: Pin<&mut Self>, action: QString, ok: bool, message: QString);

        /// 分享创建 / 撤销结束：`action` 为 `create` 或 `revoke`
        #[qsignal]
        #[cxx_name = "shareActionDone"]
        fn share_action_done(self: Pin<&mut Self>, action: QString, ok: bool, message: QString);

        /// 请求界面跳到传输任务页并定位这个条目的那一行（只对已占用的条目发）
        #[qsignal]
        #[cxx_name = "locateTransfer"]
        fn locate_transfer(self: Pin<&mut Self>, source_id: QString);

        /// 离线任务的目标文件夹路径与内容都已就绪，界面现在可以切到文件页
        #[qsignal]
        #[cxx_name = "taskFolderReady"]
        fn task_folder_ready(self: Pin<&mut Self>);

        /// 离线任务产物的父目录与列表已就绪，界面切到文件页并选中目标
        #[qsignal]
        #[cxx_name = "taskLocationReady"]
        fn task_location_ready(self: Pin<&mut Self>, target_id: QString);

        /// 资料库里的文件夹与内容已就绪，界面现在可以切到文件页。
        #[qsignal]
        #[cxx_name = "libraryFolderReady"]
        fn library_folder_ready(self: Pin<&mut Self>);

        /// 资料库右键定位成功后切到文件页，并在目标父目录中选中条目。
        #[qsignal]
        #[cxx_name = "libraryLocationReady"]
        fn library_location_ready(self: Pin<&mut Self>, target_id: QString);

        #[qinvokable]
        #[cxx_name = "login"]
        fn login(
            self: Pin<&mut Self>,
            username: QString,
            password: QString,
            captcha_token: QString,
        );

        #[qinvokable]
        #[cxx_name = "openCaptchaPage"]
        fn open_captcha_page(self: &Self);

        #[qinvokable]
        #[cxx_name = "signOut"]
        fn sign_out(self: Pin<&mut Self>);

        #[qinvokable]
        #[cxx_name = "warmUp"]
        fn warm_up(self: Pin<&mut Self>);

        /// 用户切换页面 / 目录时清掉上一条提醒，让底栏回到页面上下文。
        #[qinvokable]
        #[cxx_name = "clearStatusMessage"]
        fn clear_status_message(self: Pin<&mut Self>);

        #[qinvokable]
        #[cxx_name = "pump"]
        fn pump(self: Pin<&mut Self>);

        #[qinvokable]
        #[cxx_name = "refreshFiles"]
        fn refresh_files(self: Pin<&mut Self>);

        /// 页面重新可见时调用：缓存新鲜则不请求，过期则后台校准。
        #[qinvokable]
        #[cxx_name = "ensureFilesFresh"]
        fn ensure_files_fresh(self: Pin<&mut Self>);

        /// 请求某条文件的缩略图。QML 只传 id，远程地址和缓存路径都留在 Rust 侧。
        #[qinvokable]
        #[cxx_name = "requestThumbnail"]
        fn request_thumbnail(self: Pin<&mut Self>, id: QString);

        /// 已缓存缩略图的本地 URI；未命中返回空串。
        #[qinvokable]
        #[cxx_name = "thumbnailPath"]
        fn thumbnail_path(self: &Self, id: QString) -> QString;

        #[qinvokable]
        #[cxx_name = "refreshTrash"]
        fn refresh_trash(self: Pin<&mut Self>);

        /// 页面重新可见：缓存新鲜时直接显示，过期才静默后台校准。
        #[qinvokable]
        #[cxx_name = "ensureTrashFresh"]
        fn ensure_trash_fresh(self: Pin<&mut Self>);

        /// 从回收站还原选中条目（支持超过一批的数量，会按 100 条分批）
        #[qinvokable]
        #[cxx_name = "untrashEntries"]
        fn untrash_entries(self: Pin<&mut Self>, ids: QStringList);

        /// 从回收站彻底删除选中条目，不可恢复
        #[qinvokable]
        #[cxx_name = "deleteTrashEntries"]
        fn delete_trash_entries(self: Pin<&mut Self>, ids: QStringList);

        /// 彻底清空当前账号回收站
        #[qinvokable]
        #[cxx_name = "clearTrash"]
        fn clear_trash(self: Pin<&mut Self>);

        #[qinvokable]
        #[cxx_name = "openDir"]
        fn open_dir(self: Pin<&mut Self>, id: QString);

        #[qinvokable]
        #[cxx_name = "openParent"]
        fn open_parent(self: Pin<&mut Self>);

        #[qinvokable]
        #[cxx_name = "openCrumb"]
        fn open_crumb(self: Pin<&mut Self>, index: i32);

        #[qinvokable]
        #[cxx_name = "refreshTasks"]
        fn refresh_tasks(self: Pin<&mut Self>);

        /// 页面重新可见：任务列表由后台轮询维护，缓存新鲜时不再额外请求。
        #[qinvokable]
        #[cxx_name = "ensureTasksFresh"]
        fn ensure_tasks_fresh(self: Pin<&mut Self>);

        #[qinvokable]
        #[cxx_name = "addTask"]
        fn add_task(self: Pin<&mut Self>, url: QString);

        /// 提交离线任务并保存到指定云端目录；空目录 id 表示根目录
        #[qinvokable]
        #[cxx_name = "addTaskToFolder"]
        fn add_task_to_folder(self: Pin<&mut Self>, url: QString, folder_id: QString);

        #[qinvokable]
        #[cxx_name = "deleteTask"]
        fn delete_task(self: Pin<&mut Self>, id: QString);

        /// 清理所有已结束的离线任务记录；云端文件保留。
        #[qinvokable]
        #[cxx_name = "clearFinishedTasks"]
        fn clear_finished_tasks(self: Pin<&mut Self>);

        /// 播放已完成的离线任务产物（id 是产物文件 id）。
        #[qinvokable]
        #[cxx_name = "requestTaskPlay"]
        fn request_task_play(self: Pin<&mut Self>, id: QString);

        /// 从离线任务跳到文件页并打开已完成产物文件夹，同时补齐真实面包屑。
        #[qinvokable]
        #[cxx_name = "openTaskFolder"]
        fn open_task_folder(self: Pin<&mut Self>, id: QString) -> bool;

        /// 从离线任务跳到产物所在父目录，并在文件页选中目标文件 / 文件夹。
        #[qinvokable]
        #[cxx_name = "revealTaskEntry"]
        fn reveal_task_entry(self: Pin<&mut Self>, id: QString) -> bool;

        /// 读取分享管理页列表（进入页面 / 手动刷新）
        #[qinvokable]
        #[cxx_name = "refreshShares"]
        fn refresh_shares(self: Pin<&mut Self>);

        /// 页面重新可见：分享缓存新鲜时不再请求，过期才后台校准。
        #[qinvokable]
        #[cxx_name = "ensureSharesFresh"]
        fn ensure_shares_fresh(self: Pin<&mut Self>);

        /// 读取资料库某一页；`kind` 只接受 recent-saved / playback-history / starred。
        #[qinvokable]
        #[cxx_name = "refreshLibrary"]
        fn refresh_library(self: Pin<&mut Self>, kind: QString);

        /// 页面重新可见时调用：先恢复该集合缓存，过期再静默后台校准。
        #[qinvokable]
        #[cxx_name = "ensureLibraryFresh"]
        fn ensure_library_fresh(self: Pin<&mut Self>, kind: QString);

        /// 从资料库打开文件夹；普通文件与可播放文件不使用这个入口。
        #[qinvokable]
        #[cxx_name = "openLibraryEntry"]
        fn open_library_entry(self: Pin<&mut Self>, id: QString);

        /// 从资料库跳到文件页，加载目标父目录并选中文件或文件夹。
        #[qinvokable]
        #[cxx_name = "revealLibraryEntry"]
        fn reveal_library_entry(self: Pin<&mut Self>, id: QString) -> bool;

        /// 创建一个可包含多个文件 / 文件夹的分享。
        #[qinvokable]
        #[cxx_name = "createShare"]
        fn create_share(
            self: Pin<&mut Self>,
            file_ids: QStringList,
            expiration_days: i32,
            need_password: bool,
        );

        /// 批量撤销分享记录（不删除源文件）。
        #[qinvokable]
        #[cxx_name = "revokeShares"]
        fn revoke_shares(self: Pin<&mut Self>, share_ids: QStringList);

        #[qinvokable]
        #[cxx_name = "requestPlay"]
        fn request_play(self: Pin<&mut Self>, id: QString);

        #[qinvokable]
        #[cxx_name = "playSelected"]
        fn play_selected(self: Pin<&mut Self>, index: i32);

        #[qinvokable]
        #[cxx_name = "cancelPlay"]
        fn cancel_play(self: Pin<&mut Self>);

        /// 提示条上的「取消」：中止取链请求，或关掉还没出窗口的 mpv
        #[qinvokable]
        #[cxx_name = "cancelPlayLoading"]
        fn cancel_play_loading(self: Pin<&mut Self>);

        /// 提示条上的「关闭」：清掉错误态
        #[qinvokable]
        #[cxx_name = "dismissPlayError"]
        fn dismiss_play_error(self: Pin<&mut Self>);

        /// 提示条自己收起回退提示时调用（清掉那一份文案，下次回退才能再弹）
        #[qinvokable]
        #[cxx_name = "dismissPlayNotice"]
        fn dismiss_play_notice(self: Pin<&mut Self>);

        /// 真退出前调用：关闭所有还在播的 mpv / VLC（缩托盘不调）
        #[qinvokable]
        #[cxx_name = "shutdownPlayers"]
        fn shutdown_players(self: &Self);

        /// 退出前调用：注销托盘 D-Bus 名字，避免面板留下点不动的僵尸图标
        #[qinvokable]
        #[cxx_name = "shutdownTray"]
        fn shutdown_tray(self: &Self);

        #[qinvokable]
        #[cxx_name = "setPreferredQuality"]
        fn set_preferred_quality(self: Pin<&mut Self>, value: QString);

        #[qinvokable]
        #[cxx_name = "setPlayer"]
        fn set_player(self: Pin<&mut Self>, command: QString, args: QString);

        #[qinvokable]
        #[cxx_name = "setPlayerBackend"]
        fn set_player_backend(self: Pin<&mut Self>, value: QString);

        #[qinvokable]
        #[cxx_name = "resetPlayer"]
        fn reset_player(self: Pin<&mut Self>);

        #[qinvokable]
        #[cxx_name = "setNotifyEnabled"]
        fn set_notify_enabled(self: Pin<&mut Self>, enabled: bool);

        #[qinvokable]
        #[cxx_name = "setAutoSubtitles"]
        fn set_auto_subtitles(self: Pin<&mut Self>, enabled: bool);

        #[qinvokable]
        #[cxx_name = "setCloseToTray"]
        fn set_close_to_tray(self: Pin<&mut Self>, enabled: bool);

        /// 切换文件类内容页共用的显示方式："list" / "grid"
        #[qinvokable]
        #[cxx_name = "setContentView"]
        fn set_content_view(self: Pin<&mut Self>, value: QString);

        #[qinvokable]
        #[cxx_name = "setSorting"]
        fn set_sorting(self: Pin<&mut Self>, field: QString, ascending: bool);

        #[qinvokable]
        #[cxx_name = "saveWindowGeometry"]
        fn save_window_geometry(self: Pin<&mut Self>, width: i32, height: i32, maximized: bool);

        /// 下载一个文件或整个文件夹（文件夹轮到时递归读目录；同一条目已有批次时只跳转）
        #[qinvokable]
        #[cxx_name = "downloadEntry"]
        fn download_entry(self: Pin<&mut Self>, id: QString);

        /// 批量下载：每个条目各自一批，按顺序排队（多选右键菜单走这里）
        #[qinvokable]
        #[cxx_name = "downloadEntries"]
        fn download_entries(self: Pin<&mut Self>, ids: QStringList);

        /// 这个条目是否已在排队 / 准备 / 下载中（右键菜单文案用）
        #[qinvokable]
        #[cxx_name = "isDownloadPending"]
        fn is_download_pending(self: &Self, id: QString) -> bool;

        /// 这个文件是否有可续传的断点（右键菜单显示「继续下载」）
        #[qinvokable]
        #[cxx_name = "hasPartialDownload"]
        fn has_partial_download(self: &Self, id: QString) -> bool;

        #[qinvokable]
        #[cxx_name = "cancelDownloads"]
        fn cancel_downloads(self: Pin<&mut Self>);

        /// 取消传输页里所有下载与上传；上传没有断点，重试只能从头开始
        #[qinvokable]
        #[cxx_name = "cancelAllTransfers"]
        fn cancel_all_transfers(self: Pin<&mut Self>);

        /// 清空完成 / 失败 / 已取消的整批记录；进行中、排队与已中断保留
        #[qinvokable]
        #[cxx_name = "clearFinishedTransfers"]
        fn clear_finished_transfers(self: Pin<&mut Self>);

        /// 重试一条失败 / 已中断的传输记录（文件级回到原下载目录；文件夹整批重来）
        #[qinvokable]
        #[cxx_name = "retryTransfer"]
        fn retry_transfer(self: Pin<&mut Self>, key: QString);

        /// 重试全部失败记录；文件夹内的失败子文件逐条重试
        #[qinvokable]
        #[cxx_name = "retryFailedTransfers"]
        fn retry_failed_transfers(self: Pin<&mut Self>);

        /// 从历史里移除一条已结束 / 已中断的顶层记录（不删除本地文件或断点）
        #[qinvokable]
        #[cxx_name = "removeTransfer"]
        fn remove_transfer(self: Pin<&mut Self>, key: QString) -> bool;

        /// 取消一条正在排队 / 准备 / 下载中的记录。
        ///
        /// 顶层文件与文件夹总账行取消整批；文件夹子文件只取消自己。
        #[qinvokable]
        #[cxx_name = "cancelTransfer"]
        fn cancel_transfer(self: Pin<&mut Self>, key: QString);

        /// 暂停一个顶层传输批次（排队中立即暂停，运行中等待线程收尾）
        #[qinvokable]
        #[cxx_name = "pauseTransfer"]
        fn pause_transfer(self: Pin<&mut Self>, key: QString);

        /// 继续一个已暂停的顶层传输批次
        #[qinvokable]
        #[cxx_name = "resumeTransfer"]
        fn resume_transfer(self: Pin<&mut Self>, key: QString);

        /// 在文件管理器里显示一条已完成记录的落盘文件
        #[qinvokable]
        #[cxx_name = "revealTransfer"]
        fn reveal_transfer(self: Pin<&mut Self>, key: QString);

        /// 展开某个文件夹批次（key 无效 / 没有子文件时不会激活）
        #[qinvokable]
        #[cxx_name = "openTransferFolder"]
        fn open_transfer_folder(self: Pin<&mut Self>, key: QString);

        /// 收起当前文件夹批次的内联子行
        #[qinvokable]
        #[cxx_name = "closeTransferFolder"]
        fn close_transfer_folder(self: Pin<&mut Self>);

        #[qinvokable]
        #[cxx_name = "openDownloadDir"]
        fn open_download_dir(self: &Self);

        #[qinvokable]
        #[cxx_name = "setDownloadDir"]
        fn set_download_dir(self: Pin<&mut Self>, path: QString);

        #[qinvokable]
        #[cxx_name = "setDownloadParallel"]
        fn set_download_parallel(self: Pin<&mut Self>, value: i32);

        #[qinvokable]
        #[cxx_name = "setDownloadConnections"]
        fn set_download_connections(self: Pin<&mut Self>, value: i32);

        #[qinvokable]
        #[cxx_name = "setDownloadRateMib"]
        fn set_download_rate_mib(self: Pin<&mut Self>, value: i32);

        #[qinvokable]
        #[cxx_name = "setAutoResumeDownloads"]
        fn set_auto_resume_downloads(self: Pin<&mut Self>, enabled: bool);

        /// 通过 XDG Desktop Portal 调起系统原生目录选择器
        #[qinvokable]
        #[cxx_name = "chooseDownloadDir"]
        fn choose_download_dir(self: Pin<&mut Self>);

        /// 通过 XDG Desktop Portal 多选本地文件，上传到调用时所在的云端目录
        #[qinvokable]
        #[cxx_name = "chooseUploadFiles"]
        fn choose_upload_files(self: Pin<&mut Self>, parent_id: QString, parent_name: QString);

        /// 通过 XDG Desktop Portal 选择一个本地文件夹；多个文件夹请拖入文件页
        #[qinvokable]
        #[cxx_name = "chooseUploadFolders"]
        fn choose_upload_folders(self: Pin<&mut Self>, parent_id: QString, parent_name: QString);

        /// 接收文件管理器拖入的本地文件，上传到拖入时所在的云端目录
        #[qinvokable]
        #[cxx_name = "uploadDroppedFiles"]
        fn upload_dropped_files(
            self: Pin<&mut Self>,
            parent_id: QString,
            parent_name: QString,
            uris: QStringList,
        );

        /// 给条目改名（当前列表先更新，失败自动回滚）
        #[qinvokable]
        #[cxx_name = "renameEntry"]
        fn rename_entry(self: Pin<&mut Self>, id: QString, name: QString) -> bool;

        /// 在当前目录下新建文件夹
        #[qinvokable]
        #[cxx_name = "createFolder"]
        fn create_folder(self: Pin<&mut Self>, name: QString) -> bool;

        /// 把条目移入回收站（可恢复，界面上要二次确认）。支持多选批量。
        #[qinvokable]
        #[cxx_name = "trashEntries"]
        fn trash_entries(self: Pin<&mut Self>, ids: QStringList) -> bool;

        /// 批量加星标 / 取消星标；当前列表先更新，失败自动回滚。
        #[qinvokable]
        #[cxx_name = "setStarred"]
        fn set_starred(self: Pin<&mut Self>, ids: QStringList, starred: bool);

        /// 资料库单条加星标 / 取消星标；已加星标页取消后立即移除该行。
        #[qinvokable]
        #[cxx_name = "setLibraryStarred"]
        fn set_library_starred(self: Pin<&mut Self>, id: QString, starred: bool);

        /// 把条目移动到目标文件夹（空 id = 根目录）
        #[qinvokable]
        #[cxx_name = "moveEntries"]
        fn move_entries(
            self: Pin<&mut Self>,
            ids: QStringList,
            target_id: QString,
            target_name: QString,
        ) -> bool;

        /// 把条目复制到目标文件夹（空 id = 根目录）
        #[qinvokable]
        #[cxx_name = "copyEntries"]
        fn copy_entries(
            self: Pin<&mut Self>,
            ids: QStringList,
            target_id: QString,
            target_name: QString,
        ) -> bool;

        /// 当前目录栈（根 → 当前）：`[{ id, name }]`，选择器据此确定起点
        #[qinvokable]
        #[cxx_name = "currentPath"]
        fn current_path(self: &Self) -> QVariantList;

        /// 读取目标选择器某一层的子文件夹（独立状态，不干扰当前目录列表）
        #[qinvokable]
        #[cxx_name = "loadMoveTargets"]
        fn load_move_targets(self: Pin<&mut Self>, parent_id: QString);

        /// 关闭选择器：丢弃在途结果
        #[qinvokable]
        #[cxx_name = "cancelMoveTargets"]
        fn cancel_move_targets(self: Pin<&mut Self>);

        /// 搜索：先在当前目录已加载的条目里本地筛（0 请求）
        #[qinvokable]
        #[cxx_name = "searchLocal"]
        fn search_local(self: Pin<&mut Self>, query: QString);

        /// 搜索：遍历整个网盘（结果先到先显示，可随时停止）
        #[qinvokable]
        #[cxx_name = "searchAll"]
        fn search_all(self: Pin<&mut Self>);

        /// 搜索：停止全盘扫描（已扫出的结果保留）
        #[qinvokable]
        #[cxx_name = "stopSearch"]
        fn stop_search(self: Pin<&mut Self>);

        /// 搜索：退出搜索视图，回到当前目录
        #[qinvokable]
        #[cxx_name = "exitSearch"]
        fn exit_search(self: Pin<&mut Self>);

        /// 搜索：进入结果里的文件夹（面包屑仍是真实层级）
        #[qinvokable]
        #[cxx_name = "searchOpenFolder"]
        fn search_open_folder(self: Pin<&mut Self>, id: QString);

        /// 搜索：进入条目所在的父文件夹
        #[qinvokable]
        #[cxx_name = "searchRevealFolder"]
        fn search_reveal_folder(self: Pin<&mut Self>, id: QString);
    }
}

use cxx_qt_lib::{QString, QStringList, QVariant};
use kichi_core::MyShare;

/// Qt 的 `QVariantList`（即 `QList<QVariant>` 的别名）。
///
/// cxx-qt-lib 只导出 `QList<QVariant>`，其 ExternType Id 是 `QList<QVariant>`；
/// cxx 桥接里的类型别名要求 Rust 类型 Id 与声明名一致，因此包一层
/// `#[repr(transparent)]` 让桥接可以直接使用 Qt 的类型名。
#[repr(transparent)]
#[derive(Default, Clone)]
pub(crate) struct QVariantList(cxx_qt_lib::QList<QVariant>);

unsafe impl cxx::ExternType for QVariantList {
    type Id = cxx::type_id!("QVariantList");
    // 与 cxx-qt-lib 的 QList 保持一致：布局简单、可整体搬移，析构仍走 Rust 的 Drop
    type Kind = cxx::kind::Trivial;
}

impl QVariantList {
    pub(crate) fn append(&mut self, value: QVariant) {
        self.0.append(value);
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.len() == 0
    }

    /// 列表长度（测试与内部断言用；QList 的 len 是 isize）。
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        usize::try_from(self.0.len()).unwrap_or(0)
    }

    /// 按下标取元素（测试断言用）。
    #[cfg(test)]
    pub(crate) fn at(&self, index: usize) -> Option<&QVariant> {
        let index = isize::try_from(index).ok()?;
        self.0.get(index)
    }
}

mod actions;
mod backend;
mod downloads;
mod file_actions;
mod file_optimistic;
mod file_targets;
mod files;
mod files_cache;
mod folder_picker;
mod library;
mod notifications;
mod play;
mod play_history;
mod play_prepare;
mod queue;
mod queue_collect;
mod queue_worker;
mod reveal;
mod search;
mod settings;
mod shares;
mod tasks;
mod tasks_submit;
mod thumbnails;
mod transfers;
mod transfers_actions;
mod transfers_detail;
mod transfers_history;
mod transfers_rate;
mod transfers_view;
mod trash;
mod tray;
mod tray_icon;
mod upload_queue;
mod upload_scan;
mod upload_worker;
mod uploads;

use backend::Backend;
use files::Crumb;

/// QObject `Kichi` 的 Rust 载体。
pub struct KichiRust {
    pub(crate) auth_state: QString,
    pub(crate) status_message: QString,
    pub(crate) login_running: bool,
    pub(crate) session_restoring: bool,
    pub(crate) captcha_required: bool,
    pub(crate) account: QString,
    pub(crate) display_name: QString,
    pub(crate) avatar_path: QString,
    pub(crate) quota_used_text: QString,
    pub(crate) quota_limit_text: QString,
    pub(crate) quota_loading: bool,
    pub(crate) quota_ratio: f64,
    pub(crate) quota_vip: bool,
    /// 当前目录的列表项（QML 直接当 model 用）
    pub(crate) files: QVariantList,
    /// 当前目录 id（根为空串）：滚动记忆按它做 key
    pub(crate) current_dir_id: QString,
    /// 每次成功列目录 +1：界面据此做一次性滚动恢复
    pub(crate) files_revision: i32,
    /// 访问过的目录快照：进入已有目录先恢复，过期再后台校准。
    pub(crate) files_cache: files_cache::FilesCache,
    /// 任意目录请求在途；自动校准不改变任何可见加载属性。
    pub(crate) files_request_running: bool,
    /// 用户手动刷新已有目录。
    pub(crate) files_manual_refreshing: bool,
    /// 回收站表格（QML 直接当 model 用）
    pub(crate) trash_rows: QVariantList,
    /// 回收站领域对象：保留完整列表与总数，不依赖 QML 传来的显示数据
    pub(crate) trash_list: Vec<kichi_core::TrashEntry>,
    pub(crate) trash_loading: bool,
    /// 任意回收站请求在途；与可见加载态分开，自动校准不碰按钮。
    pub(crate) trash_request_running: bool,
    pub(crate) trash_loaded: bool,
    /// 最近一次回收站请求开始 / 完成的时刻：切页时按 TTL 决定是否校准。
    pub(crate) trash_refresh_at: Option<std::time::Instant>,
    pub(crate) trash_error: QString,
    /// 回收站列表真正替换时才 +1
    pub(crate) trash_revision: i32,
    /// 回收站请求代数：退出登录后旧结果作废
    pub(crate) trash_epoch: u64,
    /// 分批还原 / 彻底删除的进度状态
    pub(crate) trash_action: Option<trash::TrashActionState>,
    /// 文件页有恢复操作完成，下次显示时重新拉取当前目录
    pub(crate) files_stale: bool,
    /// 当前文件写操作的乐观快照；一次只允许一个写操作。
    pub(crate) file_mutation: Option<file_optimistic::FileMutation>,
    /// 新建成功但还没换到官方 id 的临时文件夹（允许连续新建多个）。
    pub(crate) unresolved_create: Vec<file_optimistic::PendingCreate>,
    /// 临时条目 id：目录校准前不可进入 / 当成真实云端条目使用。
    pub(crate) pending_entry_ids: std::collections::HashSet<String>,
    /// 临时文件夹 id 的单调序号。
    pub(crate) file_mutation_seq: u64,
    /// 面包屑显示名（根 → 当前）
    pub(crate) breadcrumb: QStringList,
    /// 当前目录的条目（openDir 查名字用，避免 QML 传来路不明的显示名）
    pub(crate) entries: Vec<kichi_core::FileEntry>,
    /// 正在下载的缩略图 id：同一行反复重建时只发一次请求。
    pub(crate) thumbnail_requests: std::collections::HashSet<String>,
    /// 缩略图请求代数：退出登录后丢弃旧账号在途结果。
    pub(crate) thumbnail_epoch: u64,
    /// 目录栈；根目录 id 为空串
    pub(crate) crumbs: Vec<Crumb>,
    /// 进 / 退目录前的目录栈快照：列表失败就回滚，面包屑别指着一个没读到的目录
    pub(crate) crumb_backup: Option<Vec<Crumb>>,
    /// 有目录列表请求在途（防止重复点击）
    pub(crate) listing: bool,
    /// 正在从离线任务定位并打开的产物文件夹 id
    pub(crate) task_folder_loading_id: QString,
    /// 列表在途时又收到刷新请求：本次完成后再补一次，不能静默丢掉
    pub(crate) refresh_pending: bool,
    /// 补发的那次刷新是否来自用户按钮；自动校准始终为 false。
    pub(crate) refresh_pending_manual: bool,
    /// 目录请求代数：退出登录 / 重登后旧结果作废
    pub(crate) list_epoch: u64,
    /// 当前离线任务（QML 直接当 model 用）
    pub(crate) tasks: QVariantList,
    /// 当前任务的领域对象（通知判断、删除确认用）
    pub(crate) task_list: Vec<kichi_core::OfflineTask>,
    /// 有任务请求在途
    pub(crate) tasks_loading: bool,
    /// 正在逐条删除已结束任务记录
    pub(crate) tasks_clearing: bool,
    /// 至少成功读过一次任务列表；空列表也算有效缓存。
    pub(crate) tasks_loaded: bool,
    /// 任意任务列表请求在途；后台轮询和切页校准不会改变可见加载态。
    pub(crate) tasks_request_running: bool,
    /// 任务列表在途时又收到强制刷新：本轮结束后补一次
    pub(crate) task_refresh_pending: bool,
    /// 最近一次任务列表请求开始 / 成功刷新的时刻：手动 / 切页刷新据此做短冷却
    pub(crate) task_refresh_at: Option<std::time::Instant>,
    /// 任务请求代数：退出登录 / 重登后旧结果作废
    pub(crate) tasks_epoch: u64,
    /// 分享管理列表（QML 直接当 model 用）
    pub(crate) shares: QVariantList,
    /// 分享列表请求在途
    pub(crate) shares_loading: bool,
    /// 至少成功读过一次分享列表；空列表也算有效缓存。
    pub(crate) shares_loaded: bool,
    /// 任意分享列表请求在途；自动校准不改变可见加载态。
    pub(crate) shares_request_running: bool,
    /// 最近一次成功读取的分享领域对象，用于判断数据是否真的变化。
    pub(crate) share_list: Vec<MyShare>,
    /// 列表在途时又要求强制刷新（创建 / 撤销完成）：本轮结束后补一次
    pub(crate) share_refresh_pending: bool,
    /// 分享列表短冷却（切页 / 手动刷新合并请求）
    pub(crate) share_refresh_at: Option<std::time::Instant>,
    /// 分享列表请求代数：退出登录 / 重登后旧结果作废
    pub(crate) shares_epoch: u64,
    /// 资料库三页共用的当前模型（QML 直接当 model 用）
    pub(crate) library_rows: QVariantList,
    /// 当前模型对应的资料库页面键值
    pub(crate) library_kind: QString,
    /// 当前资料库条目（播放查找、缩略图与文件夹打开共用）
    pub(crate) library_entries: Vec<kichi_core::LibraryEntry>,
    /// 三个资料库集合各自的最近一次成功快照，切页时先复用。
    pub(crate) library_cache: std::collections::HashMap<&'static str, library::LibraryCache>,
    pub(crate) library_loading: bool,
    /// 任意资料库请求在途；自动校准不改变可见加载态。
    pub(crate) library_request_running: bool,
    pub(crate) library_loaded: bool,
    pub(crate) library_error: QString,
    /// 同一资料库页最近一次请求开始 / 成功的时刻：自动与手动刷新共用短冷却
    pub(crate) library_refresh_at: Option<std::time::Instant>,
    /// 文件写操作期间点过手动刷新：写操作收尾后补发
    pub(crate) library_refresh_pending: Option<kichi_core::LibraryKind>,
    /// 集合真正替换时 +1
    pub(crate) library_revision: i32,
    pub(crate) library_data_revision: i32,
    /// 资料库请求代数：退出登录 / 切页后旧结果作废
    pub(crate) library_epoch: u64,
    /// 创建 / 撤销分享的写请求在途（防连点）
    pub(crate) share_action_running: bool,
    /// 分享写请求代数：退出登录后旧结果作废
    pub(crate) share_action_epoch: u64,
    /// 最近一次创建成功的链接 / 密码 / 有效期文案
    pub(crate) share_created_url: QString,
    pub(crate) share_created_pass_code: QString,
    pub(crate) share_created_expiration: QString,
    /// 上一轮仍在进行中的任务（用于识别「刚结束」并只通知一次）
    pub(crate) known_running: std::collections::HashMap<String, bool>,
    /// 已经通知过的任务 id
    pub(crate) notified: std::collections::HashSet<String>,
    /// 播放菜单（QML 当 model 用：label / original）。固定四项，不随网络变化
    pub(crate) play_menu: QVariantList,
    /// 菜单标题（文件名）
    pub(crate) play_title: QString,
    /// 菜单对应的文件 id（选中某一档后才用它取直链）
    pub(crate) play_id: String,
    /// 已选中档位、正在取直链（此时菜单关闭**不是**取消播放）
    pub(crate) play_pending: bool,
    /// 取链或等播放器就绪：对应文件缩略图显示加载状态
    pub(crate) play_loading: bool,
    /// 正在加载的文件 id
    pub(crate) play_loading_id: QString,
    /// 正在加载的文件名
    pub(crate) play_loading_name: QString,
    /// 已开播会话：请求代数 → 文件 id。
    ///
    /// 用代数作 key，而不是只存一个 id：kichi 允许上一个视频继续播放时再开新的；
    /// 旧会话退出只能清掉它自己，不能误删新会话的「播放中」标记。
    pub(crate) active_play_sessions: std::collections::BTreeMap<u64, String>,
    /// 去重后的已开播文件 id，QML 直接消费
    pub(crate) playing_ids: QStringList,
    /// 非空 = 提示条显示错误态（播放失败）
    pub(crate) play_error: QString,
    /// 非空 = 提示条短提示（档位回退说明），界面几秒后自动收起
    pub(crate) play_notice: QString,
    /// 本次加载的取消信号（取链 + 等窗口共用；取消即中止）
    ///
    /// 不额外记「窗口是否已经出现」：监控线程自己知道，`Cancel` 置真即中止，
    /// 界面侧只需要 `play_loading` 一个布尔量。
    pub(crate) play_cancel: Option<std::sync::Arc<crate::session::download::Cancel>>,
    /// 播放请求代数：退出登录 / 重登后旧结果作废
    pub(crate) play_epoch: u64,
    /// 官方播放历史的上报队列（同一文件串行，旧请求不会覆盖新进度）。
    pub(crate) play_progress: play_history::ProgressReporter,
    /// 点播放时快照的字幕计划（菜单开着时目录刷新不影响本次播放）
    pub(crate) play_subtitle_plan: crate::session::subtitles::SubtitlePlan,
    /// 持久化设置（启动时读一次，改动即落盘）
    pub(crate) settings: crate::settings::Settings,
    /// 清晰度偏好（"ask" / "original" / "transcode"，QML 展示用）
    pub(crate) preferred_quality: QString,
    /// 播放器预设（"mpv" / "vlc" / "custom"）
    pub(crate) player_backend: QString,
    /// 自定义播放器命令 / 参数（空串 = 用默认）
    pub(crate) player_command: QString,
    pub(crate) player_args: QString,
    /// 内置默认播放器命令 / 参数（设置页展示与「恢复默认」参考）
    pub(crate) player_default_command: QString,
    pub(crate) player_default_args: QString,
    pub(crate) notify_enabled: bool,
    /// 播放时自动加载同目录字幕（只对 mpv 生效）
    pub(crate) auto_subtitles: bool,
    pub(crate) close_to_tray: bool,
    /// 托盘是否真的起来了（失败时界面禁用「关闭到托盘」）
    pub(crate) tray_available: bool,
    /// 托盘要求显示窗口的计数（QML 监听变化后 show/raise）
    pub(crate) tray_show_requested: i32,
    /// 托盘要求退出
    pub(crate) quit_requested: bool,
    /// 文件类内容页共用显示方式（枚举给持久化与同步用；字符串给 QML）
    pub(crate) content_view: crate::settings::ContentViewMode,
    pub(crate) content_view_text: QString,
    /// 排序字段（枚举给重排用；字符串 `sort_field_text` 给 QML）
    pub(crate) sort_field: crate::settings::SortField,
    pub(crate) sort_field_text: QString,
    pub(crate) sort_ascending: bool,
    pub(crate) window_width: i32,
    pub(crate) window_height: i32,
    pub(crate) window_maximized: bool,
    pub(crate) settings_path: QString,
    /// 令牌存储后端（KWallet / 降级文件），设置页只读展示
    pub(crate) storage_backend: QString,
    /// 传输任务记录（跨重启保留，见 `docs/adr/0014`；QML 直接当 model 用）
    pub(crate) transfers: QVariantList,
    /// 记录结构版本（QML 据此决定要不要重读 `transfers`）
    pub(crate) transfer_revision: i32,
    /// 正在下载文件的实时进度（各文件行 + 文件夹父行聚合；进度节拍只刷新它）
    pub(crate) transfer_live: QVariantList,
    /// 当前展开文件夹批次的子文件列表（内联子行专用）
    pub(crate) transfer_detail: QVariantList,
    /// 文件夹详情是否打开
    pub(crate) transfer_detail_active: bool,
    /// 进行中（排队 + 准备中 + 下载中）的传输条数：侧栏徽标
    pub(crate) download_active: i32,
    /// 全部历史里有没有可清的已结束批次（不按日期收窄）
    pub(crate) transfer_has_finished: bool,
    /// 是否存在可批量重试的失败记录
    pub(crate) transfer_has_retryable: bool,
    /// 有批次在准备（读目录）或执行；QML 不再拿它禁用下载入口
    pub(crate) download_running: bool,
    /// 排队中的批次数（不含正在跑的那一批）
    pub(crate) download_queued: i32,
    /// 排队中的上传文件数（不含正在上传的那一个）
    pub(crate) upload_queued: i32,
    /// 正在上传的文件数（串行队列，只会是 0 或 1）
    pub(crate) upload_active: i32,
    /// 下载目录的展示值（空串 = 用默认，界面上显示实际路径）
    pub(crate) download_dir: QString,
    pub(crate) download_parallel: i32,
    pub(crate) download_connections: i32,
    pub(crate) download_rate_mib: i32,
    pub(crate) auto_resume_downloads: bool,
    /// 启动时自动续跑标记是否已经处理过（每进程一次）
    pub(crate) auto_resume_checked: bool,
    /// 有文件操作（改名 / 新建 / 删除 / 移动 / 复制）在途，界面上禁用相关入口
    pub(crate) file_action_running: bool,
    /// 移动 / 复制选择器当前层的文件夹
    pub(crate) move_targets: QVariantList,
    pub(crate) move_targets_loading: bool,
    pub(crate) move_targets_error: QString,
    /// 文件夹列表真正替换时才 +1
    pub(crate) move_targets_revision: i32,
    /// 选择器请求代数：快速进出目录时旧结果作废
    pub(crate) move_targets_epoch: u64,
    /// 搜索视图是否打开（打开时文件页显示结果而不是当前目录）
    pub(crate) search_active: bool,
    /// 当前搜索关键词（QML 输入框双向绑定的那一份）
    pub(crate) search_query: QString,
    /// 搜索结果（QML 直接当 model 用）
    pub(crate) search_results: QVariantList,
    /// 全盘扫描已扫过的条目数
    pub(crate) search_scanned: i32,
    /// 命中条数（= `search_results` 的长度，QML 不必自己数）
    pub(crate) search_hits: i32,
    /// 全盘扫描是否已结束（扫完 / 停止 / 异常）
    pub(crate) search_settled: bool,
    /// 搜索状态行文案（扫描进度或结束三态的解释）
    pub(crate) search_status: QString,
    /// 已在 `search_results` 里显示的条数（上限 [`SEARCH_LIST_LIMIT`]）
    pub(crate) search_displayed: usize,
    /// 命中数超过显示上限（界面据此提示「只显示前 N 条」）
    pub(crate) search_truncated: bool,
    /// 在途扫描：代数（停止 / 退出搜索 / 登出都 +1，旧结果丢弃）与取消信号
    pub(crate) search_epoch: u64,
    pub(crate) search_cancel: Option<std::sync::Arc<crate::session::download::Cancel>>,
    /// 搜索命中的领域数据（结果清单里的 id → 条目 / 路径）
    pub(crate) search_index: std::collections::HashMap<String, crate::session::SearchHit>,
    /// 人机验证地址（不做成属性：属性变化靠 QML 读状态消息）
    pub(crate) captcha_url: String,
    pub(crate) backend: Backend,
}

impl Default for KichiRust {
    fn default() -> Self {
        let mut breadcrumb = QStringList::default();
        breadcrumb.append(QString::from(files::ROOT_NAME));

        let mut object = Self {
            auth_state: QString::from("signedOut"),
            status_message: QString::default(),
            login_running: false,
            session_restoring: false,
            captcha_required: false,
            account: QString::default(),
            display_name: QString::default(),
            avatar_path: QString::default(),
            quota_used_text: QString::default(),
            quota_limit_text: QString::default(),
            quota_loading: false,
            quota_ratio: -1.0,
            quota_vip: false,
            files: QVariantList::default(),
            current_dir_id: QString::default(),
            files_revision: 0,
            files_cache: files_cache::FilesCache::default(),
            files_request_running: false,
            files_manual_refreshing: false,
            trash_rows: QVariantList::default(),
            trash_list: Vec::new(),
            trash_loading: false,
            trash_request_running: false,
            trash_loaded: false,
            trash_refresh_at: None,
            trash_error: QString::default(),
            trash_revision: 0,
            trash_epoch: 0,
            trash_action: None,
            files_stale: false,
            file_mutation: None,
            unresolved_create: Vec::new(),
            pending_entry_ids: std::collections::HashSet::new(),
            file_mutation_seq: 0,
            breadcrumb,
            entries: Vec::new(),
            thumbnail_requests: std::collections::HashSet::new(),
            thumbnail_epoch: 0,
            crumbs: vec![Crumb::root()],
            crumb_backup: None,
            listing: false,
            task_folder_loading_id: QString::default(),
            refresh_pending: false,
            refresh_pending_manual: false,
            list_epoch: 0,
            tasks: QVariantList::default(),
            task_list: Vec::new(),
            tasks_loading: false,
            tasks_clearing: false,
            tasks_loaded: false,
            tasks_request_running: false,
            task_refresh_pending: false,
            task_refresh_at: None,
            tasks_epoch: 0,
            shares: QVariantList::default(),
            shares_loading: false,
            shares_loaded: false,
            shares_request_running: false,
            share_list: Vec::new(),
            share_refresh_pending: false,
            share_refresh_at: None,
            shares_epoch: 0,
            library_rows: QVariantList::default(),
            library_kind: QString::default(),
            library_entries: Vec::new(),
            library_cache: std::collections::HashMap::new(),
            library_loading: false,
            library_request_running: false,
            library_loaded: false,
            library_error: QString::default(),
            library_refresh_at: None,
            library_refresh_pending: None,
            library_revision: 0,
            library_data_revision: 0,
            library_epoch: 0,
            share_action_running: false,
            share_action_epoch: 0,
            share_created_url: QString::default(),
            share_created_pass_code: QString::default(),
            share_created_expiration: QString::default(),
            known_running: std::collections::HashMap::new(),
            notified: std::collections::HashSet::new(),
            play_menu: QVariantList::default(),
            play_title: QString::default(),
            play_id: String::new(),
            play_pending: false,
            play_loading: false,
            play_loading_id: QString::default(),
            play_loading_name: QString::default(),
            active_play_sessions: std::collections::BTreeMap::new(),
            playing_ids: QStringList::default(),
            play_error: QString::default(),
            play_notice: QString::default(),
            play_cancel: None,
            play_epoch: 0,
            play_progress: play_history::ProgressReporter::default(),
            play_subtitle_plan: crate::session::subtitles::SubtitlePlan::None,
            settings: crate::settings::Settings::default(),
            preferred_quality: QString::from("ask"),
            player_backend: QString::from("mpv"),
            player_command: QString::default(),
            player_args: QString::default(),
            player_default_command: QString::from(crate::player::DEFAULT_PLAYER),
            player_default_args: QString::from(crate::player::DEFAULT_ARGS),
            notify_enabled: true,
            auto_subtitles: true,
            close_to_tray: false,
            tray_available: false,
            tray_show_requested: 0,
            quit_requested: false,
            content_view: crate::settings::ContentViewMode::List,
            content_view_text: QString::from("list"),
            sort_field: crate::settings::SortField::Name,
            sort_field_text: QString::from("name"),
            sort_ascending: true,
            window_width: 0,
            window_height: 0,
            window_maximized: false,
            settings_path: QString::from(
                crate::settings::default_path().to_string_lossy().as_ref(),
            ),
            storage_backend: QString::default(),
            transfers: QVariantList::default(),
            transfer_revision: 0,
            transfer_live: QVariantList::default(),
            transfer_detail: QVariantList::default(),
            transfer_detail_active: false,
            download_active: 0,
            transfer_has_finished: false,
            transfer_has_retryable: false,
            download_running: false,
            download_queued: 0,
            upload_queued: 0,
            upload_active: 0,
            download_dir: QString::default(),
            download_parallel: i32::try_from(crate::session::limits::DEFAULT_PARALLEL).unwrap_or(3),
            download_connections: i32::try_from(crate::session::limits::DEFAULT_CONNECTIONS)
                .unwrap_or(4),
            download_rate_mib: 0,
            auto_resume_downloads: true,
            auto_resume_checked: false,
            file_action_running: false,
            move_targets: QVariantList::default(),
            move_targets_loading: false,
            move_targets_error: QString::default(),
            move_targets_revision: 0,
            move_targets_epoch: 0,
            search_active: false,
            search_query: QString::default(),
            search_results: QVariantList::default(),
            search_scanned: 0,
            search_hits: 0,
            search_settled: true,
            search_status: QString::default(),
            search_displayed: 0,
            search_truncated: false,
            search_epoch: 0,
            search_cancel: None,
            search_index: std::collections::HashMap::new(),
            captcha_url: String::new(),
            backend: Backend::new(crate::session::store_path()),
        };
        object.load_settings();
        // 启动时读回传输历史（首次 sync_transfers 之前；批次号从历史最大值之后接着走）
        downloads::load_history(&mut object);
        object.apply_restored();
        object
    }
}

impl KichiRust {
    /// 按 id 找条目的名字：先看当前目录，再看搜索结果。
    ///
    /// 搜索视图里点播放 / 下载时，条目只存在于结果清单里，
    /// 只看 `entries` 会误报「已不在当前列表中」。
    pub(crate) fn entry_name(&self, id: &str) -> Option<String> {
        self.entries
            .iter()
            .find(|entry| entry.id == id)
            .or_else(|| self.search_index.get(id).map(|hit| &hit.entry))
            .or_else(|| {
                self.library_entries
                    .iter()
                    .map(|entry| &entry.entry)
                    .find(|entry| entry.id == id)
            })
            .map(|entry| entry.name.clone())
    }

    /// 播放菜单对应的文件 id（菜单开着时才有值）。
    pub(crate) fn play_target(&self) -> Option<String> {
        (!self.play_id.is_empty()).then(|| self.play_id.clone())
    }

    /// 同 [`KichiRust::entry_name`]，按 id 找整个条目（下载要类别与名字）。
    pub(crate) fn find_entry(&self, id: &str) -> Option<kichi_core::FileEntry> {
        self.entries
            .iter()
            .find(|entry| entry.id == id)
            .or_else(|| self.search_index.get(id).map(|hit| &hit.entry))
            .or_else(|| {
                self.library_entries
                    .iter()
                    .map(|entry| &entry.entry)
                    .find(|entry| entry.id == id)
            })
            .cloned()
    }

    /// 构造时读设置：属性先落地，再让界面看到（QML 构造时属性已可读）。
    fn load_settings(&mut self) {
        let path = crate::settings::default_path();
        self.settings_path = QString::from(path.to_string_lossy().as_ref());
        let (settings, warning) = crate::settings::load(&path);
        self.settings = settings;
        crate::session::limits::configure(
            self.settings.download_parallel,
            self.settings.download_connections,
            self.settings.download_rate_mib,
        );
        self.sync_settings_initial();
        if let Some(warning) = warning {
            // 设置坏掉不该拦住使用：提示走状态栏，其余照默认跑
            self.status_message = QString::from(warning.as_str());
        }
    }

    /// 启动时把设置刷到属性上（不写盘、不触发错误提示）。
    fn sync_settings_initial(&mut self) {
        let settings = self.settings.clone();
        self.preferred_quality = QString::from(settings.preferred_quality.as_str());
        self.player_backend = QString::from(settings.player_backend.as_str());
        self.player_command = QString::from(settings.player_command.as_str());
        self.player_args = QString::from(settings.player_args.as_str());
        self.notify_enabled = settings.notify_enabled;
        self.auto_subtitles = settings.auto_subtitles;
        self.close_to_tray = settings.close_to_tray;
        self.content_view = settings.content_view;
        self.content_view_text = QString::from(settings.content_view.as_str());
        self.sort_field = settings.sort_field;
        self.sort_field_text = QString::from(settings.sort_field.as_str());
        self.sort_ascending = settings.sort_ascending;
        self.window_width = settings.window_width;
        self.window_height = settings.window_height;
        self.window_maximized = settings.window_maximized;
        // 下载目录属性展示「实际生效的路径」，空串表示用默认
        self.download_dir = QString::from(settings.download_dir.as_str());
        self.download_parallel = i32::try_from(settings.download_parallel)
            .unwrap_or(crate::session::limits::DEFAULT_PARALLEL as i32);
        self.download_connections = i32::try_from(settings.download_connections)
            .unwrap_or(crate::session::limits::DEFAULT_CONNECTIONS as i32);
        self.download_rate_mib = i32::try_from(settings.download_rate_mib).unwrap_or(i32::MAX);
        self.auto_resume_downloads = settings.auto_resume_downloads;
        let backend = self.backend.restored_backend();
        self.storage_backend = QString::from(match backend {
            Some(crate::session::Backend::Keychain) => "系统钥匙串（KWallet）",
            Some(crate::session::Backend::File) => "降级文件（钥匙串不可用）",
            None => "尚未使用",
        });
        // 上次开着「关闭到托盘」：这次启动就得把托盘拉起来，
        // 否则用户以为缩进托盘了、实际没有，关窗会直接退出
        if settings.close_to_tray {
            self.tray_available = tray::start();
        }
    }

    /// 构造时恢复会话（只读本地存储 + 必要时刷新一次令牌）。
    fn apply_restored(&mut self) {
        use crate::session::State;
        let state = self.backend.state();
        self.auth_state = QString::from(actions::state_name(state));
        if state == State::SignedIn {
            self.session_restoring = true;
            self.status_message = QString::from("正在恢复会话……");
            self.account = QString::from(self.backend.session.email().unwrap_or_default().as_str());
            self.display_name = QString::from(
                self.backend
                    .session
                    .display_name()
                    .unwrap_or_default()
                    .as_str(),
            );
            if let Some(path) = self.backend.session.cached_avatar_path() {
                self.avatar_path = QString::from(reveal::file_uri(&path).as_str());
            }
            return;
        }
        let notice = self.backend.restored_notice().to_owned();
        self.status_message = QString::from(if notice.is_empty() {
            self.backend.storage()
        } else {
            notice
        });
    }
}

/// 打开系统默认浏览器（不引入额外依赖；xdg-open 是 Linux 桌面标配）。
pub(crate) fn open_in_browser(url: &str) {
    let spawned = std::process::Command::new("xdg-open")
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    if let Err(err) = spawned {
        eprintln!("[kichi] 打开浏览器失败：{err}");
    }
}
