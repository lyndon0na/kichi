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
        /// 当月云下载额度（离线保存）；服务端没下发时为空串
        #[qproperty(QString, quota_transfer_cloud, cxx_name = "quotaTransferCloud", READ, NOTIFY = changed)]
        /// 当月下行额度（播放 / 查看 / 下载）
        #[qproperty(QString, quota_transfer_download, cxx_name = "quotaTransferDownload", READ, NOTIFY = changed)]
        /// 当月上传额度
        #[qproperty(QString, quota_transfer_upload, cxx_name = "quotaTransferUpload", READ, NOTIFY = changed)]
        /// 额度重置时间（下月 1 日 00:00 SGT）
        #[qproperty(QString, quota_transfer_reset, cxx_name = "quotaTransferReset", READ, NOTIFY = changed)]
        /// 当月额度占用比例 0..1；`-1` = 没有上限，界面不画进度条
        #[qproperty(f64, quota_transfer_cloud_ratio, cxx_name = "quotaTransferCloudRatio", READ, NOTIFY = changed)]
        #[qproperty(f64, quota_transfer_download_ratio, cxx_name = "quotaTransferDownloadRatio", READ, NOTIFY = changed)]
        #[qproperty(f64, quota_transfer_upload_ratio, cxx_name = "quotaTransferUploadRatio", READ, NOTIFY = changed)]
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
        /// `files` 按当前筛选条件裁过的版本：文件页列表 / 网格直接用它
        #[qproperty(QVariantList, filtered_files, cxx_name = "filteredFiles", READ, NOTIFY = changed)]
        #[qproperty(QStringList, breadcrumb, READ, NOTIFY = changed)]
        /// 与 `breadcrumb` 并列的目录 id（根目录为空串）：内部拖拽落到面包屑时用它
        #[qproperty(QStringList, breadcrumb_ids, cxx_name = "breadcrumbIds", READ, NOTIFY = changed)]
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
        /// 本地索引正在全量建库（资料库与选择器据此暂缓读索引）
        #[qproperty(bool, index_building, cxx_name = "indexBuilding", READ, NOTIFY = changed)]
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
        /// 数据来源说明（星标页在本地索引 / 云端之间切换时显示）。
        #[qproperty(QString, library_status, cxx_name = "libraryStatus", READ, NOTIFY = changed)]
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
        /// 正在解析 / 打开 / 浏览别人的分享
        #[qproperty(bool, share_open_busy, cxx_name = "shareOpenBusy", READ, NOTIFY = changed)]
        /// 分享已打开、当前目录已就绪（弹窗切到浏览态）
        #[qproperty(bool, share_open_ready, cxx_name = "shareOpenReady", READ, NOTIFY = changed)]
        /// 打开 / 浏览失败的提示（链接无效、需要密码、链接失效、网络错误）
        #[qproperty(QString, share_open_error, cxx_name = "shareOpenError", READ, NOTIFY = changed)]
        /// 上次失败是「需要访问密码」：弹窗据此聚焦密码框
        #[qproperty(bool, share_open_needs_pass_code, cxx_name = "shareOpenNeedsPassCode", READ, NOTIFY = changed)]
        #[qproperty(QString, share_open_title, cxx_name = "shareOpenTitle", READ, NOTIFY = changed)]
        /// 分享者昵称（可能为空）
        #[qproperty(QString, share_open_owner, cxx_name = "shareOpenOwner", READ, NOTIFY = changed)]
        /// 「N 个条目 · 到期时间」摘要
        #[qproperty(QString, share_open_meta, cxx_name = "shareOpenMeta", READ, NOTIFY = changed)]
        /// 分享当前层的条目（QML 直接当 model 用：id / name / kind / isFolder / sizeText）
        #[qproperty(QVariantList, share_open_rows, cxx_name = "shareOpenRows", READ, NOTIFY = changed)]
        /// 浏览路径（根 → 当前），元素为 { id, name }
        #[qproperty(QVariantList, share_open_path, cxx_name = "shareOpenPath", READ, NOTIFY = changed)]
        /// 分页达到上限，只显示了前一部分条目
        #[qproperty(bool, share_open_truncated, cxx_name = "shareOpenTruncated", READ, NOTIFY = changed)]
        /// 转存请求在途
        #[qproperty(bool, share_save_running, cxx_name = "shareSaveRunning", READ, NOTIFY = changed)]
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
        /// 图片预览正在取详情 / 下载原图
        #[qproperty(bool, image_preview_loading, cxx_name = "imagePreviewLoading", READ, NOTIFY = changed)]
        /// 图片预览失败原因；空串 = 没有错误
        #[qproperty(QString, image_preview_error, cxx_name = "imagePreviewError", READ, NOTIFY = changed)]
        /// 已下载到本地的预览图 `file://` URI；空串 = 还没准备好
        #[qproperty(QString, image_preview_path, cxx_name = "imagePreviewPath", READ, NOTIFY = changed)]
        /// 内嵌播放画面已出图：界面据此显示整窗视频浮层
        #[qproperty(bool, embed_active, cxx_name = "embedActive", READ, NOTIFY = changed)]
        /// 当前内嵌播放的标题（浮层文字）
        #[qproperty(QString, embed_title, cxx_name = "embedTitle", READ, NOTIFY = changed)]
        /// 每次内嵌会话 Ready 后 +1：同一个窗口切下一集时刷新章节 / 进度
        #[qproperty(i32, embed_serial, cxx_name = "embedSerial", READ, NOTIFY = changed)]
        /// 本次播放的文件夹连播队列（QML 当 model 用：id / name / current）
        #[qproperty(QVariantList, play_queue, cxx_name = "playQueue", READ, NOTIFY = changed)]
        /// 当前集在队列里的位置；-1 = 没有队列或未定位
        #[qproperty(i32, play_queue_index, cxx_name = "playQueueIndex", READ, NOTIFY = changed)]
        /// 播完自动播放同文件夹下一集（设置页可关）
        #[qproperty(bool, auto_play_next, cxx_name = "autoPlayNext", READ, NOTIFY = changed)]
        /// 正在同一个播放窗口里切下一集（取链 / 等新画面）
        #[qproperty(bool, queue_advancing, cxx_name = "queueAdvancing", READ, NOTIFY = changed)]
        /// 内嵌播放器音量（0–100，持久化）
        #[qproperty(i32, embed_volume, cxx_name = "embedVolume", READ, NOTIFY = changed)]
        /// 内嵌播放器是否静音（持久化）
        #[qproperty(bool, embed_muted, cxx_name = "embedMuted", READ, NOTIFY = changed)]
        /// 字幕字号（持久化）
        #[qproperty(i32, embed_sub_font_size, cxx_name = "embedSubFontSize", READ, NOTIFY = changed)]
        /// 字幕垂直位置（0 顶部 / 100 底部）
        #[qproperty(i32, embed_sub_pos, cxx_name = "embedSubPos", READ, NOTIFY = changed)]
        /// 字幕文字颜色（`#AARRGGBB`）
        #[qproperty(QString, embed_sub_color, cxx_name = "embedSubColor", READ, NOTIFY = changed)]
        /// 字幕描边粗细
        #[qproperty(i32, embed_sub_outline_size, cxx_name = "embedSubOutlineSize", READ, NOTIFY = changed)]
        /// 字幕描边 / 边框颜色
        #[qproperty(QString, embed_sub_outline_color, cxx_name = "embedSubOutlineColor", READ, NOTIFY = changed)]
        #[qproperty(QString, preferred_quality, cxx_name = "preferredQuality", READ, NOTIFY = changed)]
        /// 播放器预设："embedded" / "mpv" / "vlc" / "custom"
        #[qproperty(QString, player_backend, cxx_name = "playerBackend", READ, NOTIFY = changed)]
        /// 是否运行在 Flatpak 沙箱里（只读；沙箱内只提供内嵌播放器）
        #[qproperty(bool, flatpak, cxx_name = "flatpak", READ, NOTIFY = changed)]
        #[qproperty(QString, player_command, cxx_name = "playerCommand", READ, NOTIFY = changed)]
        #[qproperty(QString, player_args, cxx_name = "playerArgs", READ, NOTIFY = changed)]
        #[qproperty(QString, player_default_command, cxx_name = "playerDefaultCommand", READ, NOTIFY = changed)]
        #[qproperty(QString, player_default_args, cxx_name = "playerDefaultArgs", READ, NOTIFY = changed)]
        #[qproperty(bool, notify_enabled, cxx_name = "notifyEnabled", READ, NOTIFY = changed)]
        /// 播放时自动加载同目录字幕（只对 mpv 生效）
        #[qproperty(bool, auto_subtitles, cxx_name = "autoSubtitles", READ, NOTIFY = changed)]
        /// 内嵌播放首选音轨语言："auto" / "zh" / "yue" / "en" / "ja" / "ko"
        #[qproperty(QString, player_audio_lang, cxx_name = "playerAudioLang", READ, NOTIFY = changed)]
        /// 内嵌播放首选字幕语言
        #[qproperty(QString, player_sub_lang, cxx_name = "playerSubLang", READ, NOTIFY = changed)]
        /// 内嵌播放读取自己的 embed-mpv.conf
        #[qproperty(bool, player_mpv_conf, cxx_name = "playerMpvConf", READ, NOTIFY = changed)]
        /// embed-mpv.conf 的实际路径（可能被 KICHI_EMBED_MPV_CONF 覆盖）
        #[qproperty(QString, embed_mpv_conf_path, cxx_name = "embedMpvConfPath", READ, NOTIFY = changed)]
        /// 内嵌播放加载自己的 embed-input.conf
        #[qproperty(bool, player_input_conf, cxx_name = "playerInputConf", READ, NOTIFY = changed)]
        /// 内嵌播放使用 Vulkan 渲染（关闭 = OpenGL）
        #[qproperty(bool, player_vulkan, cxx_name = "playerVulkan", READ, NOTIFY = changed)]
        /// embed-input.conf 的实际路径（可能被 KICHI_EMBED_INPUT_CONF 覆盖）
        #[qproperty(QString, embed_input_conf_path, cxx_name = "embedInputConfPath", READ, NOTIFY = changed)]
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
        /// 排队中的上传批次数（不含正在上传的批次）
        #[qproperty(i32, upload_queued, cxx_name = "uploadQueued", READ, NOTIFY = changed)]
        /// 正在上传的批次数
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
        #[qproperty(bool, auto_resume_uploads, cxx_name = "autoResumeUploads", READ, NOTIFY = changed)]
        /// 本机保存目录树索引与页面快照；关闭时不读不写本地库
        #[qproperty(bool, local_index_enabled, cxx_name = "localIndexEnabled", READ, NOTIFY = changed)]
        /// 本地索引已成功建库、可承担全盘本地搜索；搜索框据此切换占位文案。
        /// 开关打开但尚未建好 / 正在首次建库 / 读取失败时都为 false。
        #[qproperty(bool, local_index_ready, cxx_name = "localIndexReady", READ, NOTIFY = changed)]
        #[qproperty(i32, upload_parallel, cxx_name = "uploadParallel", READ, NOTIFY = changed)]
        #[qproperty(i32, upload_rate_mib, cxx_name = "uploadRateMib", READ, NOTIFY = changed)]
        #[qproperty(bool, file_action_running, cxx_name = "fileActionRunning", READ, NOTIFY = changed)]
        /// 正在被写操作占用的条目 id；同一 id 在完成前不能再发起第二个写操作
        #[qproperty(QStringList, file_action_ids, cxx_name = "fileActionIds", READ, NOTIFY = changed)]
        /// 有回收站写操作（还原 / 彻底删除 / 清空 / 文件页移入）在途；不影响其它页面
        #[qproperty(bool, trash_action_running, cxx_name = "trashActionRunning", READ, NOTIFY = changed)]
        /// 正在等待官方异步任务完成的整库清空
        #[qproperty(bool, trash_clearing, cxx_name = "trashClearing", READ, NOTIFY = changed)]
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
        /// 结果来源："index"（本地索引全盘）/ "fallback"（输入阶段只筛当前目录）/ "network"（全盘扫描）；
        /// 空串 = 没有搜索。只有 network 才显示扫描进度与停止按钮。
        #[qproperty(QString, search_source, cxx_name = "searchSource", READ, NOTIFY = changed)]
        /// 已勾选的类型线名（folder/file/video/audio/image/document/archive/other）；空 = 全部
        #[qproperty(QStringList, search_filter_types, cxx_name = "searchFilterTypes", READ, NOTIFY = changed)]
        /// 已勾选的修改时间线名（7d/30d/1y）；空 = 不限
        #[qproperty(QStringList, search_filter_modifieds, cxx_name = "searchFilterModifieds", READ, NOTIFY = changed)]
        /// 已勾选的大小线名（lt10m/10m-100m/100m-1g/gt1g）；空 = 不限
        #[qproperty(QStringList, search_filter_sizes, cxx_name = "searchFilterSizes", READ, NOTIFY = changed)]
        /// 激活的筛选条件组数（0–3），筛选按钮上显示
        #[qproperty(i32, search_filter_active_count, cxx_name = "searchFilterActiveCount", READ, NOTIFY = changed)]
        /// 状态行摘要，例如「视频 · 最近 7 天」；没有筛选时为空串
        #[qproperty(QString, search_filter_summary, cxx_name = "searchFilterSummary", READ, NOTIFY = changed)]
        /// 磁链解析出的文件清单（QML 直接当 model 用）
        #[qproperty(QVariantList, magnet_files, cxx_name = "magnetFiles", READ, NOTIFY = changed)]
        /// 磁链解析在途
        #[qproperty(bool, magnet_loading, cxx_name = "magnetFilesLoading", READ, NOTIFY = changed)]
        /// 磁链解析失败原因；空串 = 没有错误
        #[qproperty(QString, magnet_error, cxx_name = "magnetFilesError", READ, NOTIFY = changed)]
        /// 单根目录名 / 磁链 `dn=`；多文件保存时用它当容器文件夹名
        #[qproperty(QString, magnet_root_name, cxx_name = "magnetRootName", READ, NOTIFY = changed)]
        /// 弹窗摘要：「N 个文件 · 大小；M 个可秒传」
        #[qproperty(QString, magnet_summary, cxx_name = "magnetSummary", READ, NOTIFY = changed)]
        /// 秒传请求在途
        #[qproperty(bool, magnet_saving, cxx_name = "magnetSaving", READ, NOTIFY = changed)]
        /// 秒传进度 0..1；`magnetSaving` 为 false 时归零
        #[qproperty(f64, magnet_save_progress, cxx_name = "magnetSaveProgress", READ, NOTIFY = changed)]
        type Kichi = super::KichiRust;

        /// 统一变更通知：所有属性都挂这一个信号
        #[qsignal]
        fn changed(self: Pin<&mut Self>);

        /// 缩略图已落到本地缓存（`path` 是可直接给 Image 的 file:// URI）。
        /// 失败时 `path` 为空串，QML 退回文件图标。
        #[qsignal]
        #[cxx_name = "thumbnailReady"]
        fn thumbnail_ready(self: Pin<&mut Self>, id: QString, path: QString);

        /// 一次文件操作结束：动作、成功与否、给用户看的提示、涉及的目标 id。
        /// QML 用最后一项只处理自己那批条目的完成事件，避免并发写串台。
        #[qsignal]
        #[cxx_name = "fileActionDone"]
        fn file_action_done(
            self: Pin<&mut Self>,
            action: QString,
            ok: bool,
            message: QString,
            ids: QStringList,
        );

        /// 新建文件夹的临时行已插入：界面立即定位并强调，不等待网络结果。
        #[qsignal]
        #[cxx_name = "folderCreateStarted"]
        fn folder_create_started(self: Pin<&mut Self>, id: QString);

        /// 临时文件夹已校准到真实云端 id：界面只把强调原位交给新 id，不再次滚动。
        #[qsignal]
        #[cxx_name = "folderCreated"]
        fn folder_created(self: Pin<&mut Self>, id: QString);

        /// 分享创建 / 撤销结束：`action` 为 `create` 或 `revoke`
        #[qsignal]
        #[cxx_name = "shareActionDone"]
        fn share_action_done(self: Pin<&mut Self>, action: QString, ok: bool, message: QString);

        /// 别人分享的条目转存结束：`ok = false` 时 `message` 是失败原因
        #[qsignal]
        #[cxx_name = "shareRestoreDone"]
        fn share_restore_done(self: Pin<&mut Self>, ok: bool, message: QString);

        /// 已取到文件直链：`url` 是带签名的临时地址，仅供界面立即写剪贴板。
        /// 不写日志、不存属性；失败不加班信号，直接走底栏提示。
        #[qsignal]
        #[cxx_name = "fileLinkReady"]
        fn file_link_ready(self: Pin<&mut Self>, id: QString, url: QString);

        /// 文件 / 文件夹属性读取结束：`ok` 为假时 `message` 是原因，其余字段为空。
        /// 成功时各字段已是可直接显示的文本，缺失值统一是「—」。
        #[qsignal]
        #[cxx_name = "entryPropertiesReady"]
        fn entry_properties_ready(
            self: Pin<&mut Self>,
            id: QString,
            ok: bool,
            message: QString,
            name: QString,
            kind: QString,
            size: QString,
            created: QString,
            modified: QString,
            entry_id: QString,
            path: QString,
        );

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

        /// 离线任务完成并写入某个目录：文件页若正显示该目录，可静默刷新。
        #[qsignal]
        #[cxx_name = "offlineTaskFolderChanged"]
        fn offline_task_folder_changed(self: Pin<&mut Self>, folder_id: QString);

        /// 本地 `.torrent` 解析成功：名称、磁力链接与「文件数 · 大小」摘要。
        #[qsignal]
        #[cxx_name = "torrentParsed"]
        fn torrent_parsed(self: Pin<&mut Self>, name: QString, magnet: QString, detail: QString);

        /// 本地 `.torrent` 解析失败；界面在添加任务弹窗里显示原因。
        #[qsignal]
        #[cxx_name = "torrentParseFailed"]
        fn torrent_parse_failed(self: Pin<&mut Self>, message: QString);

        /// 磁链文件清单解析结束：成功后弹窗读取 `magnetFiles` 等属性。
        /// `can_fallback` 为 false 时（登录态 / 验证 / 限流）不要自动改交云下载；
        /// `fallback_links` 是解析失败或没有可秒传文件的磁链，应单独回落为普通云下载。
        #[qsignal]
        #[cxx_name = "magnetPreviewFinished"]
        fn magnet_preview_finished(
            self: Pin<&mut Self>,
            ok: bool,
            message: QString,
            can_fallback: bool,
            fallback_links: QStringList,
        );

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

        /// 离线任务完成后的静默目录校准：不显示手动刷新反馈。
        #[qinvokable]
        #[cxx_name = "refreshFilesBackground"]
        fn refresh_files_background(self: Pin<&mut Self>);

        /// 窗口重新获得焦点：触发一次 canary 探针（自带冷却与单飞）。
        #[qinvokable]
        #[cxx_name = "onWindowActivated"]
        fn on_window_activated(self: Pin<&mut Self>);

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

        /// 从回收站彻底删除选中条目，不可恢复；接受请求后先在本地移除
        #[qinvokable]
        #[cxx_name = "deleteTrashEntries"]
        fn delete_trash_entries(self: Pin<&mut Self>, ids: QStringList) -> bool;

        /// 清空当前账号回收站；接受请求后先清空本地列表，完成后发 `fileActionDone`
        #[qinvokable]
        #[cxx_name = "clearTrash"]
        fn clear_trash(self: Pin<&mut Self>) -> bool;

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

        /// 重新提交失败 / 状态未知的离线任务（沿用原来源链接与已知目标目录）。
        #[qinvokable]
        #[cxx_name = "retryTask"]
        fn retry_task(self: Pin<&mut Self>, id: QString);

        /// 解析拖入添加任务弹窗的 `.torrent`，结果通过 `torrentParsed` 回投。
        #[qinvokable]
        #[cxx_name = "previewTorrentFiles"]
        fn preview_torrent_files(self: Pin<&mut Self>, uris: QStringList);

        /// 通过 XDG Portal 选择 `.torrent` 文件，结果通过 `torrentParsed` 回投。
        #[qinvokable]
        #[cxx_name = "chooseTorrentFiles"]
        fn choose_torrent_files(self: Pin<&mut Self>);

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

        /// 解析并打开一段别人的分享链接 / 分享文本。
        #[qinvokable]
        #[cxx_name = "openShareLink"]
        fn open_share_link(self: Pin<&mut Self>, text: QString, pass_code: QString);

        /// 进入分享内的文件夹。
        #[qinvokable]
        #[cxx_name = "openShareFolder"]
        fn open_share_folder(self: Pin<&mut Self>, folder_id: QString, folder_name: QString);

        /// 回到分享内的上一层（顶层时无操作）。
        #[qinvokable]
        #[cxx_name = "shareOpenGoUp"]
        fn share_open_go_up(self: Pin<&mut Self>);

        /// 关闭分享弹窗并清空浏览状态（在途结果作废）。
        #[qinvokable]
        #[cxx_name = "closeShareOpen"]
        fn close_share_open(self: Pin<&mut Self>);

        /// 把选中的分享条目转存到自己的目标目录（空 id = 根目录）。
        #[qinvokable]
        #[cxx_name = "restoreShareFiles"]
        fn restore_share_files(
            self: Pin<&mut Self>,
            file_ids: QStringList,
            parent_id: QString,
        ) -> bool;

        #[qinvokable]
        #[cxx_name = "requestPlay"]
        fn request_play(self: Pin<&mut Self>, id: QString);

        #[qinvokable]
        #[cxx_name = "playSelected"]
        fn play_selected(self: Pin<&mut Self>, index: i32);

        /// 播放窗口：上一集
        #[qinvokable]
        #[cxx_name = "playPrevious"]
        fn play_previous(self: Pin<&mut Self>);

        /// 播放窗口：下一集
        #[qinvokable]
        #[cxx_name = "playNext"]
        fn play_next(self: Pin<&mut Self>);

        /// 播放列表里点某一集（队列下标）
        #[qinvokable]
        #[cxx_name = "playQueueItem"]
        fn play_queue_item(self: Pin<&mut Self>, index: i32);

        /// 设置页：播完自动下一集
        #[qinvokable]
        #[cxx_name = "setAutoPlayNext"]
        fn set_auto_play_next(self: Pin<&mut Self>, enabled: bool);

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

        /// 内嵌浮层：暂停 / 继续
        #[qinvokable]
        #[cxx_name = "embedTogglePause"]
        fn embed_toggle_pause(self: &Self);

        /// 内嵌浮层：跳到某个秒数
        #[qinvokable]
        #[cxx_name = "embedSeek"]
        fn embed_seek(self: &Self, seconds: f64);

        /// 内嵌浮层：调音量（0–100）
        #[qinvokable]
        #[cxx_name = "embedSetVolume"]
        fn embed_set_volume(self: Pin<&mut Self>, volume: i32);

        /// 内嵌浮层：静音 / 取消静音
        #[qinvokable]
        #[cxx_name = "embedToggleMute"]
        fn embed_toggle_mute(self: Pin<&mut Self>);

        /// 把当前音量 / 静音写进设置（停止调节后调用）
        #[qinvokable]
        #[cxx_name = "embedCommitAudio"]
        fn embed_commit_audio(self: Pin<&mut Self>);

        /// 内嵌浮层：关闭这次播放
        #[qinvokable]
        #[cxx_name = "embedStop"]
        fn embed_stop(self: &Self);

        /// 当前播放位置（秒）；没有会话时为 0。界面按固定节拍主动读取。
        #[qinvokable]
        #[cxx_name = "embedPosition"]
        fn embed_position(self: &Self) -> f64;

        /// 片长（秒）；还不知道时为 0
        #[qinvokable]
        #[cxx_name = "embedDuration"]
        fn embed_duration(self: &Self) -> f64;

        /// 是否处于暂停
        #[qinvokable]
        #[cxx_name = "embedPaused"]
        fn embed_paused(self: &Self) -> bool;

        /// 是否正在加载 / 缓冲（等缓存、跳转中或起播前空闲）；界面按固定节拍读取
        #[qinvokable]
        #[cxx_name = "embedBuffering"]
        fn embed_buffering(self: &Self) -> bool;

        /// 当前轨道列表：`{ id, kind: "audio" / "sub", label, selected }`
        #[qinvokable]
        #[cxx_name = "embedTracks"]
        fn embed_tracks(self: &Self) -> QVariantList;

        /// 章节列表：`{ time: 起点秒数, title }`；文件没有章节时为空
        #[qinvokable]
        #[cxx_name = "embedChapters"]
        fn embed_chapters(self: &Self) -> QVariantList;

        /// 切换音轨 / 字幕轨；`id < 0` 表示关闭字幕
        #[qinvokable]
        #[cxx_name = "embedSelectTrack"]
        fn embed_select_track(self: &Self, kind: QString, id: i32);

        /// 内嵌会话当前音量（mpv 原生键位改过也读得到）
        #[qinvokable]
        #[cxx_name = "embedCurrentVolume"]
        fn embed_current_volume(self: &Self) -> i32;

        /// 内嵌会话当前是否静音
        #[qinvokable]
        #[cxx_name = "embedCurrentMuted"]
        fn embed_current_muted(self: &Self) -> bool;

        /// 把 mpv 原生键位改过的音量 / 静音同步到界面属性（不再下发给 mpv）
        #[qinvokable]
        #[cxx_name = "embedSyncAudio"]
        fn embed_sync_audio(self: Pin<&mut Self>, volume: i32, muted: bool);

        /// 当前倍速（1.0 = 原速）
        #[qinvokable]
        #[cxx_name = "embedSpeed"]
        fn embed_speed(self: &Self) -> f64;

        /// 改倍速（百分比，50–300）并写进设置
        #[qinvokable]
        #[cxx_name = "embedSetSpeed"]
        fn embed_set_speed(self: Pin<&mut Self>, percent: i32);

        /// 拖动倍速滑块时只改会话 / mpv，不写设置（松手后再由 embedSetSpeed 提交）
        #[qinvokable]
        #[cxx_name = "embedPreviewSpeed"]
        fn embed_preview_speed(self: &Self, percent: i32);

        /// 把播放窗口收到的按键交给 mpv（mpv 原生键位）
        #[qinvokable]
        #[cxx_name = "embedKeypress"]
        fn embed_keypress(self: &Self, key: QString);

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
        #[cxx_name = "setPlayerAudioLang"]
        fn set_player_audio_lang(self: Pin<&mut Self>, value: QString);

        #[qinvokable]
        #[cxx_name = "setPlayerSubLang"]
        fn set_player_sub_lang(self: Pin<&mut Self>, value: QString);

        #[qinvokable]
        #[cxx_name = "setPlayerMpvConf"]
        fn set_player_mpv_conf(self: Pin<&mut Self>, enabled: bool);

        #[qinvokable]
        #[cxx_name = "openPlayerMpvConf"]
        fn open_player_mpv_conf(self: Pin<&mut Self>);

        #[qinvokable]
        #[cxx_name = "setPlayerInputConf"]
        fn set_player_input_conf(self: Pin<&mut Self>, enabled: bool);

        #[qinvokable]
        #[cxx_name = "openPlayerInputConf"]
        fn open_player_input_conf(self: Pin<&mut Self>);

        #[qinvokable]
        #[cxx_name = "setPlayerVulkan"]
        fn set_player_vulkan(self: Pin<&mut Self>, enabled: bool);

        #[qinvokable]
        #[cxx_name = "setSubtitleFontSize"]
        fn set_subtitle_font_size(self: Pin<&mut Self>, value: i32);

        /// 字幕垂直位置（0 顶部 / 100 底部）
        #[qinvokable]
        #[cxx_name = "setSubtitlePosition"]
        fn set_subtitle_position(self: Pin<&mut Self>, value: i32);

        #[qinvokable]
        #[cxx_name = "setSubtitleColor"]
        fn set_subtitle_color(self: Pin<&mut Self>, value: QString);

        #[qinvokable]
        #[cxx_name = "setSubtitleOutlineSize"]
        fn set_subtitle_outline_size(self: Pin<&mut Self>, value: i32);

        #[qinvokable]
        #[cxx_name = "setSubtitleOutlineColor"]
        fn set_subtitle_outline_color(self: Pin<&mut Self>, value: QString);

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

        /// 下载到…：先弹系统目录选择器，选中后按该目录入队（只对本次生效，
        /// 不改全局下载目录）；已在队列里的条目不重复入队
        #[qinvokable]
        #[cxx_name = "chooseDownloadDirForEntries"]
        fn choose_download_dir_for_entries(self: Pin<&mut Self>, ids: QStringList);

        /// 取单个文件的下载直链，成功后发 `fileLinkReady` 让界面复制到剪贴板。
        /// 文件夹没有直链，不会请求。
        #[qinvokable]
        #[cxx_name = "copyLink"]
        fn copy_link(self: Pin<&mut Self>, id: QString);

        /// 属性弹窗打开时按 id 读一次详情（文件夹同样支持），结果走
        /// `entryPropertiesReady`；失败不阻塞文件页。
        #[qinvokable]
        #[cxx_name = "requestEntryProperties"]
        fn request_entry_properties(self: Pin<&mut Self>, id: QString);

        /// 预览图片：按 id 取详情与直链，Rust 下载到本地临时文件后回填
        /// `imagePreviewPath`；失败写 `imagePreviewError`。QML 从不见远程 URL。
        #[qinvokable]
        #[cxx_name = "previewImage"]
        fn preview_image(self: Pin<&mut Self>, id: QString);

        /// 关闭图片预览：作废在途结果并删除本地临时文件。
        #[qinvokable]
        #[cxx_name = "closeImagePreview"]
        fn close_image_preview(self: Pin<&mut Self>);

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

        /// 取消传输页里所有下载与上传；上传会丢弃断点，之后只能从头重试
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

        #[qinvokable]
        #[cxx_name = "setAutoResumeUploads"]
        fn set_auto_resume_uploads(self: Pin<&mut Self>, enabled: bool);

        /// 打开 / 关闭本地目录索引与页面快照（关闭后不读写本地库，库文件保留）
        #[qinvokable]
        #[cxx_name = "setLocalIndexEnabled"]
        fn set_local_index_enabled(self: Pin<&mut Self>, enabled: bool);

        /// 取消正在进行的本地索引全量建库（开关保持开启，旧库不动）
        #[qinvokable]
        #[cxx_name = "cancelLocalIndexBuild"]
        fn cancel_local_index_build(self: Pin<&mut Self>);

        #[qinvokable]
        #[cxx_name = "setUploadParallel"]
        fn set_upload_parallel(self: Pin<&mut Self>, value: i32);

        #[qinvokable]
        #[cxx_name = "setUploadRateMib"]
        fn set_upload_rate_mib(self: Pin<&mut Self>, value: i32);

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

        /// 从文件页彻底删除条目（不经过回收站，确认框里主动勾选后才调用）
        #[qinvokable]
        #[cxx_name = "deleteEntries"]
        fn delete_entries(self: Pin<&mut Self>, ids: QStringList) -> bool;

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
        /// `force` = true 时跳过本地快照强制联网（界面「刷新」）
        #[qinvokable]
        #[cxx_name = "loadMoveTargets"]
        fn load_move_targets(self: Pin<&mut Self>, parent_id: QString, force: bool);

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

        /// 搜索：设置筛选（三组已勾选项的线名列表，空列表 = 该组不限制）。
        /// 只改状态，界面随后复用现有防抖重跑当前关键词。
        #[qinvokable]
        #[cxx_name = "setSearchFilter"]
        fn set_search_filter(
            self: Pin<&mut Self>,
            types: QStringList,
            modified: QStringList,
            sizes: QStringList,
        );

        /// 搜索：清除全部筛选（空态里的「清除筛选」）
        #[qinvokable]
        #[cxx_name = "clearSearchFilters"]
        fn clear_search_filters(self: Pin<&mut Self>);

        /// 解析磁力链接的文件清单（不创建云下载任务）。
        #[qinvokable]
        #[cxx_name = "previewMagnetFiles"]
        fn preview_magnet_files(self: Pin<&mut Self>, link: QString);

        /// 秒传选中的文件；`indices` 是 `magnetFiles` 的下标，`folder_id` 空串 = 根目录。
        #[qinvokable]
        #[cxx_name = "saveMagnetFiles"]
        fn save_magnet_files(
            self: Pin<&mut Self>,
            indices: QVariantList,
            folder_id: QString,
            keep_structure: bool,
        );
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

    /// 逐项读取（invokable 参数解析用）。
    pub(crate) fn iter(&self) -> impl Iterator<Item = &QVariant> {
        self.0.iter()
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

// 桥接实现按功能分组放进子目录，与 `kichi/qml/` 的功能目录对应。
// 模块名与模块树保持平铺（用 `#[path]` 指向实际文件），所以各实现里的
// `super::xxx` 仍指向 `cxxqt_object` 本身，不会因搬目录连锁改引用。

// 通用基础设施：后台线程、配置、通知、缩略图、文件定位、目录选择器
#[path = "cxxqt_object/common/actions.rs"]
mod actions;
#[path = "cxxqt_object/common/actions_display.rs"]
mod actions_display;
#[path = "cxxqt_object/common/actions_index.rs"]
mod actions_index;
#[path = "cxxqt_object/common/actions_status.rs"]
mod actions_status;
#[path = "cxxqt_object/common/backend.rs"]
mod backend;
#[path = "cxxqt_object/common/folder_picker.rs"]
mod folder_picker;
#[path = "cxxqt_object/common/notifications.rs"]
mod notifications;
#[path = "cxxqt_object/common/reveal.rs"]
mod reveal;
#[path = "cxxqt_object/common/thumbnails.rs"]
mod thumbnails;

// 外壳：设置模型、系统托盘
#[path = "cxxqt_object/shell/settings.rs"]
mod settings;
#[path = "cxxqt_object/shell/tray.rs"]
mod tray;
#[path = "cxxqt_object/shell/tray_icon.rs"]
mod tray_icon;

// 文件浏览与文件操作
#[path = "cxxqt_object/files/file_action_lock.rs"]
mod file_action_lock;
#[path = "cxxqt_object/files/file_actions.rs"]
mod file_actions;
#[path = "cxxqt_object/files/file_link.rs"]
mod file_link;
#[path = "cxxqt_object/files/file_optimistic.rs"]
mod file_optimistic;
#[path = "cxxqt_object/files/file_properties.rs"]
mod file_properties;
#[path = "cxxqt_object/files/file_stars.rs"]
mod file_stars;
#[path = "cxxqt_object/files/file_targets.rs"]
mod file_targets;
#[path = "cxxqt_object/files/file_transfers.rs"]
mod file_transfers;
#[path = "cxxqt_object/files/files.rs"]
mod files;
#[path = "cxxqt_object/files/files_cache.rs"]
mod files_cache;
#[path = "cxxqt_object/files/files_nav.rs"]
mod files_nav;
#[path = "cxxqt_object/files/files_view.rs"]
mod files_view;
#[path = "cxxqt_object/files/image_preview.rs"]
mod image_preview;

// 回收站
#[path = "cxxqt_object/trash/trash.rs"]
mod trash;

// 下载：调度、目录遍历、下载循环
#[path = "cxxqt_object/downloads/downloads.rs"]
mod downloads;
#[path = "cxxqt_object/downloads/pick_dir.rs"]
mod pick_dir;
#[path = "cxxqt_object/downloads/queue.rs"]
mod queue;
#[path = "cxxqt_object/downloads/queue_collect.rs"]
mod queue_collect;
#[path = "cxxqt_object/downloads/queue_worker.rs"]
mod queue_worker;

// 上传：扫描、调度、上传线程、入口
#[path = "cxxqt_object/upload/upload_queue.rs"]
mod upload_queue;
#[path = "cxxqt_object/upload/upload_scan.rs"]
mod upload_scan;
#[path = "cxxqt_object/upload/upload_worker.rs"]
mod upload_worker;
#[path = "cxxqt_object/upload/upload_worker_file.rs"]
mod upload_worker_file;
#[path = "cxxqt_object/upload/upload_worker_folder.rs"]
mod upload_worker_folder;
#[path = "cxxqt_object/upload/upload_worker_notice.rs"]
mod upload_worker_notice;
#[path = "cxxqt_object/upload/uploads.rs"]
mod uploads;

// 本机传输记录：状态机、明细、落盘、限速、视图
#[path = "cxxqt_object/transfers/transfers.rs"]
mod transfers;
#[path = "cxxqt_object/transfers/transfers_actions.rs"]
mod transfers_actions;
#[path = "cxxqt_object/transfers/transfers_detail.rs"]
mod transfers_detail;
#[path = "cxxqt_object/transfers/transfers_history.rs"]
mod transfers_history;
#[path = "cxxqt_object/transfers/transfers_rate.rs"]
mod transfers_rate;
#[path = "cxxqt_object/transfers/transfers_sync.rs"]
mod transfers_sync;
#[path = "cxxqt_object/transfers/transfers_view.rs"]
mod transfers_view;

// 离线任务：磁力解析、提交、种子
#[path = "cxxqt_object/tasks/magnets.rs"]
mod magnets;
#[path = "cxxqt_object/tasks/tasks.rs"]
mod tasks;
#[path = "cxxqt_object/tasks/tasks_submit.rs"]
mod tasks_submit;
#[path = "cxxqt_object/tasks/torrents.rs"]
mod torrents;

// 播放：取链、续播、播放列表
#[path = "cxxqt_object/playback/play.rs"]
mod play;
#[path = "cxxqt_object/playback/play_embed.rs"]
mod play_embed;
#[path = "cxxqt_object/playback/play_fetch.rs"]
mod play_fetch;
#[path = "cxxqt_object/playback/play_history.rs"]
mod play_history;
#[path = "cxxqt_object/playback/play_prepare.rs"]
mod play_prepare;
#[path = "cxxqt_object/playback/play_start.rs"]
mod play_start;
#[path = "cxxqt_object/playback/play_state.rs"]
mod play_state;
#[path = "cxxqt_object/playback/playlist.rs"]
mod playlist;
#[path = "cxxqt_object/playback/playlist_prefetch.rs"]
mod playlist_prefetch;
#[path = "cxxqt_object/playback/playlist_recovery.rs"]
mod playlist_recovery;

// 资料库
#[path = "cxxqt_object/library/library.rs"]
mod library;

// 搜索
#[path = "cxxqt_object/search/search.rs"]
mod search;
#[path = "cxxqt_object/search/search_filter.rs"]
mod search_filter;
#[path = "cxxqt_object/search/search_local.rs"]
mod search_local;

// 分享
#[path = "cxxqt_object/shares/share_open.rs"]
mod share_open;
#[path = "cxxqt_object/shares/shares.rs"]
mod shares;

use backend::Backend;
use files::Crumb;
use share_open::ShareCrumb;

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
    pub(crate) quota_transfer_cloud: QString,
    pub(crate) quota_transfer_download: QString,
    pub(crate) quota_transfer_upload: QString,
    pub(crate) quota_transfer_reset: QString,
    pub(crate) quota_transfer_cloud_ratio: f64,
    pub(crate) quota_transfer_download_ratio: f64,
    pub(crate) quota_transfer_upload_ratio: f64,
    /// 当前目录的列表项（QML 直接当 model 用）
    pub(crate) files: QVariantList,
    /// `files` 的筛选后版本（顺序与 `files` 一致，只少掉不匹配的行）
    pub(crate) filtered_files: QVariantList,
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
    /// 回收站写操作 / 请求期间收到的刷新，等收尾后再补一次
    pub(crate) trash_refresh_pending: bool,
    pub(crate) trash_error: QString,
    /// 回收站列表真正替换时才 +1
    pub(crate) trash_revision: i32,
    /// 回收站请求代数：退出登录后旧结果作废
    pub(crate) trash_epoch: u64,
    /// 同时进行的回收站写操作；键是文件动作 id
    pub(crate) trash_actions: std::collections::HashMap<u64, trash::TrashActionState>,
    /// 文件页有恢复操作完成，下次显示时重新拉取当前目录
    pub(crate) files_stale: bool,
    /// 每个文件动作各自的乐观回滚快照
    pub(crate) file_mutations: std::collections::HashMap<u64, file_optimistic::FileMutation>,
    /// 新建成功但还没换到官方 id 的临时文件夹（允许连续新建多个）。
    pub(crate) unresolved_create: Vec<file_optimistic::PendingCreate>,
    /// 临时条目 id：目录校准前不可进入 / 当成真实云端条目使用。
    pub(crate) pending_entry_ids: std::collections::HashSet<String>,
    /// 面包屑显示名（根 → 当前）
    pub(crate) breadcrumb: QStringList,
    /// 与 `breadcrumb` 一一对应的目录 id（根为空串）
    pub(crate) breadcrumb_ids: QStringList,
    /// 当前目录的条目（openDir 查名字用，避免 QML 传来路不明的显示名）
    pub(crate) entries: Vec<kichi_core::FileEntry>,
    /// 正在下载的缩略图 id：同一行反复重建时只发一次请求。
    pub(crate) thumbnail_requests: std::collections::HashSet<String>,
    /// 缩略图请求代数：退出登录后丢弃旧账号在途结果。
    pub(crate) thumbnail_epoch: u64,
    /// 图片预览：在途 / 失败原因 / 本地 `file://` URI
    pub(crate) image_preview_loading: bool,
    pub(crate) image_preview_error: QString,
    pub(crate) image_preview_path: QString,
    /// 当前预览的临时文件；关闭 / 换图 / 登出时删除
    pub(crate) image_preview_file: Option<std::path::PathBuf>,
    /// 预览请求代数：关闭 / 换图 / 登出后丢弃在途结果
    pub(crate) image_preview_epoch: u64,
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
    pub(crate) library_status: QString,
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
    /// 打开别人分享：解析 / 打开 / 浏览请求在途
    pub(crate) share_open_busy: bool,
    /// 分享元数据与当前目录已就绪
    pub(crate) share_open_ready: bool,
    /// 上次失败是否因为缺少访问密码
    pub(crate) share_open_needs_pass_code: bool,
    pub(crate) share_open_error: QString,
    pub(crate) share_open_title: QString,
    pub(crate) share_open_owner: QString,
    pub(crate) share_open_meta: QString,
    /// 分享当前层的条目（QML 当 model 用）
    pub(crate) share_open_rows: QVariantList,
    /// 浏览路径（根 → 当前）
    pub(crate) share_open_path: QVariantList,
    pub(crate) share_open_truncated: bool,
    /// 已打开的分享 id 与后续请求要带的令牌
    pub(crate) share_open_share_id: String,
    pub(crate) share_open_pass_code_token: String,
    /// 浏览栈：首项固定为分享顶层，末项为当前目录
    pub(crate) share_open_stack: Vec<ShareCrumb>,
    /// 分享打开 / 浏览请求代数：关闭弹窗或退出登录后旧结果作废
    pub(crate) share_open_epoch: u64,
    /// 转存请求在途（防连点）
    pub(crate) share_save_running: bool,
    /// 转存请求代数：关闭弹窗或退出登录后旧结果作废
    pub(crate) share_save_epoch: u64,
    /// 上一轮仍在进行中的任务（用于识别「刚结束」并只通知一次）
    pub(crate) known_running: std::collections::HashMap<String, bool>,
    /// 已经通知过的任务 id
    pub(crate) notified: std::collections::HashSet<String>,
    /// 本次会话提交的离线任务 → 目标目录 id；空串表示根目录
    pub(crate) task_destinations: std::collections::HashMap<String, String>,
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
    /// 内嵌画面已经出图（Ready 后为 true；停止 / 失败 / 取消后收起）
    pub(crate) embed_active: bool,
    /// 当前内嵌播放的标题
    pub(crate) embed_title: QString,
    /// 每次内嵌会话 Ready 后 +1：同一个窗口切下一集时刷新章节 / 进度
    pub(crate) embed_serial: i32,
    /// 本次播放的文件夹连播队列（QML 直接当 model 用）
    pub(crate) play_queue: QVariantList,
    /// 当前集在队列里的位置；-1 = 没有队列或未定位
    pub(crate) play_queue_index: i32,
    /// 播完自动播放同文件夹下一集
    pub(crate) auto_play_next: bool,
    /// 正在同一个播放窗口里切下一集（QML 加载遮罩据此显示）
    pub(crate) queue_advancing: bool,
    /// 队列 / 预取的内部状态
    pub(crate) play_queue_state: playlist::PlayQueueState,
    /// 等待 Ready 的内嵌会话代数：只有它的 Ready 能点亮浮层
    pub(crate) embed_pending_epoch: Option<u64>,
    /// 已经 Ready 且仍在播放的内嵌会话代数；切集失败时据此决定要不要收起空窗口
    pub(crate) embed_live_epoch: Option<u64>,
    /// 内嵌播放器音量（0–100；随设置持久化）
    pub(crate) embed_volume: i32,
    /// 内嵌播放器静音状态（随设置持久化）
    pub(crate) embed_muted: bool,
    /// 字幕字号 / 位置 / 颜色 / 描边（随设置持久化）
    pub(crate) embed_sub_font_size: i32,
    pub(crate) embed_sub_pos: i32,
    pub(crate) embed_sub_color: QString,
    pub(crate) embed_sub_outline_size: i32,
    pub(crate) embed_sub_outline_color: QString,
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
    /// 全量建库的取消信号（登出 / 切换账号时置真）
    pub(crate) index_build_cancel: Option<std::sync::Arc<crate::session::download::Cancel>>,
    /// 建库代数：退出登录后旧进度与旧结果作废
    pub(crate) index_build_epoch: u64,
    /// 有全量建库在后台跑（期间读路径不再同步查索引）
    pub(crate) index_building: bool,
    /// 本次会话已提示过索引错误（只提示一次）
    pub(crate) index_error_reported: bool,
    /// canary 探针在途（与全量建库单飞）
    pub(crate) index_probing: bool,
    /// 探针代数：登出 / 切换账号后旧结果作废
    pub(crate) index_probe_epoch: u64,
    /// 探针取消信号（登出 / 账号切换时置真）
    pub(crate) index_probe_cancel: Option<std::sync::Arc<crate::session::download::Cancel>>,
    /// 持久化设置（启动时读一次，改动即落盘）
    pub(crate) settings: crate::settings::Settings,
    /// 清晰度偏好（"ask" / "original" / "transcode"，QML 展示用）
    pub(crate) preferred_quality: QString,
    /// 播放器预设（"embedded" / "mpv" / "vlc" / "custom"）
    pub(crate) player_backend: QString,
    /// 是否运行在 Flatpak 沙箱里（只读；沙箱内只提供内嵌播放器）
    pub(crate) flatpak: bool,
    /// 自定义播放器命令 / 参数（空串 = 用默认）
    pub(crate) player_command: QString,
    pub(crate) player_args: QString,
    /// 内置默认播放器命令 / 参数（设置页展示与「恢复默认」参考）
    pub(crate) player_default_command: QString,
    pub(crate) player_default_args: QString,
    pub(crate) notify_enabled: bool,
    /// 播放时自动加载同目录字幕（只对 mpv 生效）
    pub(crate) auto_subtitles: bool,
    /// 内嵌播放首选语言（QML 展示用）
    pub(crate) player_audio_lang: QString,
    pub(crate) player_sub_lang: QString,
    /// 内嵌播放读取自己的 embed-mpv.conf
    pub(crate) player_mpv_conf: bool,
    pub(crate) embed_mpv_conf_path: QString,
    /// 内嵌播放加载自己的 embed-input.conf
    pub(crate) player_input_conf: bool,
    pub(crate) embed_input_conf_path: QString,
    /// 内嵌播放使用 Vulkan 渲染（关闭 = OpenGL）
    pub(crate) player_vulkan: bool,
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
    /// 令牌存储后端（系统钥匙串 / 降级文件），设置页只读展示
    pub(crate) storage_backend: QString,
    /// 传输任务记录（跨重启保留；QML 直接当 model 用）
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
    /// 正在上传的文件数（单文件批次可并发，文件夹批内也按并发上限运行）
    pub(crate) upload_active: i32,
    /// 下载目录的展示值（空串 = 用默认，界面上显示实际路径）
    pub(crate) download_dir: QString,
    pub(crate) download_parallel: i32,
    pub(crate) download_connections: i32,
    pub(crate) download_rate_mib: i32,
    pub(crate) auto_resume_downloads: bool,
    pub(crate) auto_resume_uploads: bool,
    /// 在本机保存目录树索引与页面快照（关闭时不读不写本地库）
    pub(crate) local_index_enabled: bool,
    /// 本地索引已成功建库、可承担全盘本地搜索
    pub(crate) local_index_ready: bool,
    pub(crate) upload_parallel: i32,
    pub(crate) upload_rate_mib: i32,
    /// 启动时自动续跑标记是否已经处理过（每进程一次）
    pub(crate) auto_resume_checked: bool,
    /// 有文件操作在途；具体哪些条目被占用看 `file_action_ids`
    pub(crate) file_action_running: bool,
    /// 正在被写操作占用的条目 id（QML 按目标条目置灰）
    pub(crate) file_action_ids: QStringList,
    /// 文件动作 id 的单调序号
    pub(crate) file_action_seq: u64,
    /// 当前被占用的条目 id，入口判重与界面共用这一份真相
    pub(crate) file_action_locks: std::collections::HashSet<String>,
    /// 文件动作 id → 目标条目 / 所属页面，收尾时据此释放
    pub(crate) file_action_records:
        std::collections::HashMap<u64, file_action_lock::FileActionRecord>,
    /// 有回收站写操作在途；只阻止回收站自己的刷新与清空
    pub(crate) trash_action_running: bool,
    /// 整库清空在等官方后台任务完成
    pub(crate) trash_clearing: bool,
    /// 移动 / 复制选择器当前层的文件夹
    pub(crate) move_targets: QVariantList,
    pub(crate) move_targets_loading: bool,
    pub(crate) move_targets_error: QString,
    /// 文件夹列表真正替换时才 +1
    pub(crate) move_targets_revision: i32,
    /// 选择器请求代数：快速进出目录时旧结果作废
    pub(crate) move_targets_epoch: u64,
    /// `move_targets` 属于哪个目录（网络失败时判断旧行还能不能留）
    pub(crate) move_targets_dir: String,
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
    /// 结果来源："index"（本地索引全盘）/ "fallback"（只筛当前目录）/ "network"；空串 = 没有搜索
    pub(crate) search_source: QString,
    /// 文件页常驻的筛选条件（三组线名 + 摘要）：同时裁目录列表与搜索结果，
    /// 跨目录 / 跨搜索保留，只有「清除筛选」或退出登录才清空。
    pub(crate) search_filter: kichi_core::SearchFilter,
    pub(crate) search_filter_types: QStringList,
    pub(crate) search_filter_modifieds: QStringList,
    pub(crate) search_filter_sizes: QStringList,
    pub(crate) search_filter_active_count: i32,
    pub(crate) search_filter_summary: QString,
    /// 本地索引搜索在途：单飞，最多一个后台查询
    pub(crate) search_local_running: bool,
    /// 在途期间用户又输入了关键词：只保留最后一个，完成后接着搜
    pub(crate) search_pending: Option<String>,
    /// 磁链解析出的文件行（QML 直接当 model 用）
    pub(crate) magnet_files: QVariantList,
    /// 本次解析出的磁链清单（秒传按它们建目录 / 取 GCID）
    pub(crate) magnet_listings: Vec<kichi_core::MagnetListing>,
    pub(crate) magnet_root_name: QString,
    pub(crate) magnet_summary: QString,
    pub(crate) magnet_error: QString,
    pub(crate) magnet_loading: bool,
    pub(crate) magnet_saving: bool,
    pub(crate) magnet_save_progress: f64,
    /// 磁链解析 / 秒传代数：重新解析或退出登录后旧结果作废
    pub(crate) magnet_epoch: u64,
    /// 人机验证地址（不做成属性：属性变化靠 QML 读状态消息）
    pub(crate) captcha_url: String,
    pub(crate) backend: Backend,
}

impl Default for KichiRust {
    fn default() -> Self {
        let mut breadcrumb = QStringList::default();
        breadcrumb.append(QString::from(files::ROOT_NAME));
        // 只显示根目录时 id 也占一项：两条列表始终按下标一一对应
        let mut breadcrumb_ids = QStringList::default();
        breadcrumb_ids.append(QString::default());

        // 索引开关在会话启动前就要生效：关闭时启动期不创建也不打开索引库。
        // `load_settings` 之后还会再读一次设置，这里只为拿到这一个布尔值。
        let local_index_enabled = crate::settings::load(&crate::settings::default_path())
            .0
            .local_index_enabled;

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
            quota_transfer_cloud: QString::default(),
            quota_transfer_download: QString::default(),
            quota_transfer_upload: QString::default(),
            quota_transfer_reset: QString::default(),
            quota_transfer_cloud_ratio: -1.0,
            quota_transfer_download_ratio: -1.0,
            quota_transfer_upload_ratio: -1.0,
            files: QVariantList::default(),
            filtered_files: QVariantList::default(),
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
            trash_refresh_pending: false,
            trash_error: QString::default(),
            trash_revision: 0,
            trash_epoch: 0,
            trash_actions: std::collections::HashMap::new(),
            files_stale: false,
            file_mutations: std::collections::HashMap::new(),
            unresolved_create: Vec::new(),
            pending_entry_ids: std::collections::HashSet::new(),
            breadcrumb,
            breadcrumb_ids,
            entries: Vec::new(),
            thumbnail_requests: std::collections::HashSet::new(),
            thumbnail_epoch: 0,
            image_preview_loading: false,
            image_preview_error: QString::default(),
            image_preview_path: QString::default(),
            image_preview_file: None,
            image_preview_epoch: 0,
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
            library_status: QString::default(),
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
            share_open_busy: false,
            share_open_ready: false,
            share_open_needs_pass_code: false,
            share_open_error: QString::default(),
            share_open_title: QString::default(),
            share_open_owner: QString::default(),
            share_open_meta: QString::default(),
            share_open_rows: QVariantList::default(),
            share_open_path: QVariantList::default(),
            share_open_truncated: false,
            share_open_share_id: String::new(),
            share_open_pass_code_token: String::new(),
            share_open_stack: Vec::new(),
            share_open_epoch: 0,
            share_save_running: false,
            share_save_epoch: 0,
            known_running: std::collections::HashMap::new(),
            notified: std::collections::HashSet::new(),
            task_destinations: std::collections::HashMap::new(),
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
            embed_active: false,
            embed_title: QString::default(),
            embed_serial: 0,
            play_queue: QVariantList::default(),
            play_queue_index: -1,
            auto_play_next: true,
            queue_advancing: false,
            play_queue_state: playlist::PlayQueueState::default(),
            embed_pending_epoch: None,
            embed_live_epoch: None,
            embed_volume: 100,
            embed_muted: false,
            embed_sub_font_size: i32::from(crate::settings::DEFAULT_SUB_FONT_SIZE),
            embed_sub_pos: i32::from(crate::settings::DEFAULT_SUB_POS),
            embed_sub_color: QString::from(crate::settings::DEFAULT_SUB_COLOR),
            embed_sub_outline_size: i32::from(crate::settings::DEFAULT_SUB_OUTLINE_SIZE),
            embed_sub_outline_color: QString::from(crate::settings::DEFAULT_SUB_OUTLINE_COLOR),
            play_cancel: None,
            play_epoch: 0,
            play_progress: play_history::ProgressReporter::default(),
            play_subtitle_plan: crate::session::subtitles::SubtitlePlan::None,
            index_build_cancel: None,
            index_build_epoch: 0,
            index_building: false,
            index_error_reported: false,
            index_probing: false,
            index_probe_epoch: 0,
            index_probe_cancel: None,
            settings: crate::settings::Settings::default(),
            preferred_quality: QString::from("ask"),
            player_backend: QString::from("embedded"),
            flatpak: crate::platform::flatpak_sandbox(),
            player_command: QString::default(),
            player_args: QString::default(),
            player_default_command: QString::from(crate::player::DEFAULT_PLAYER),
            player_default_args: QString::from(crate::player::DEFAULT_ARGS),
            notify_enabled: true,
            auto_subtitles: true,
            player_audio_lang: QString::from("auto"),
            player_sub_lang: QString::from("auto"),
            player_mpv_conf: true,
            embed_mpv_conf_path: QString::from(
                crate::session::paths::embed_mpv_conf_path()
                    .to_string_lossy()
                    .as_ref(),
            ),
            player_input_conf: true,
            embed_input_conf_path: QString::from(
                crate::session::paths::embed_input_conf_path()
                    .to_string_lossy()
                    .as_ref(),
            ),
            player_vulkan: true,
            close_to_tray: false,
            tray_available: false,
            tray_show_requested: 0,
            quit_requested: false,
            content_view: crate::settings::ContentViewMode::Grid,
            content_view_text: QString::from("grid"),
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
            auto_resume_uploads: false,
            local_index_enabled,
            local_index_ready: false,
            upload_parallel: i32::try_from(crate::session::limits::DEFAULT_UPLOAD_PARALLEL)
                .unwrap_or(2),
            upload_rate_mib: 0,
            auto_resume_checked: false,
            file_action_running: false,
            file_action_ids: QStringList::default(),
            file_action_seq: 0,
            file_action_locks: std::collections::HashSet::new(),
            file_action_records: std::collections::HashMap::new(),
            trash_action_running: false,
            trash_clearing: false,
            move_targets: QVariantList::default(),
            move_targets_loading: false,
            move_targets_error: QString::default(),
            move_targets_revision: 0,
            move_targets_epoch: 0,
            move_targets_dir: String::new(),
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
            search_source: QString::default(),
            search_filter: kichi_core::SearchFilter::default(),
            search_filter_types: QStringList::default(),
            search_filter_modifieds: QStringList::default(),
            search_filter_sizes: QStringList::default(),
            search_filter_active_count: 0,
            search_filter_summary: QString::default(),
            search_local_running: false,
            search_pending: None,
            magnet_files: QVariantList::default(),
            magnet_listings: Vec::new(),
            magnet_root_name: QString::default(),
            magnet_summary: QString::default(),
            magnet_error: QString::default(),
            magnet_loading: false,
            magnet_saving: false,
            magnet_save_progress: 0.0,
            magnet_epoch: 0,
            captcha_url: String::new(),
            backend: Backend::new(crate::session::store_path(), local_index_enabled),
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
        crate::session::limits::configure_uploads(
            self.settings.upload_parallel,
            self.settings.upload_rate_mib,
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
        self.auto_play_next = settings.auto_play_next;
        self.player_audio_lang = QString::from(settings.player_audio_lang.as_str());
        self.player_sub_lang = QString::from(settings.player_sub_lang.as_str());
        self.player_mpv_conf = settings.player_mpv_conf;
        self.player_input_conf = settings.player_input_conf;
        self.player_vulkan = settings.player_vulkan;
        self.embed_volume = settings.player_volume;
        self.embed_muted = settings.player_muted;
        self.embed_sub_font_size = i32::from(settings.player_sub_font_size);
        self.embed_sub_pos = i32::from(settings.player_sub_pos);
        self.embed_sub_color = QString::from(settings.player_sub_color.as_str());
        self.embed_sub_outline_size = i32::from(settings.player_sub_outline_size);
        self.embed_sub_outline_color = QString::from(settings.player_sub_outline_color.as_str());
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
        self.auto_resume_uploads = settings.auto_resume_uploads;
        self.local_index_enabled = settings.local_index_enabled;
        self.upload_parallel = i32::try_from(settings.upload_parallel)
            .unwrap_or(crate::session::limits::DEFAULT_UPLOAD_PARALLEL as i32);
        self.upload_rate_mib = i32::try_from(settings.upload_rate_mib).unwrap_or(i32::MAX);
        let backend = self.backend.restored_backend();
        self.storage_backend = QString::from(match backend {
            Some(crate::session::Backend::Keychain) => "系统钥匙串",
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
            // 容量也先用上次成功读取的值占位；后台刷新回来再静默校准。
            if let Some(quota) = self.backend.session.cached_quota() {
                let display = actions::quota_display(&quota);
                self.quota_used_text = QString::from(display.used.as_str());
                self.quota_limit_text = QString::from(display.limit.as_str());
                self.quota_ratio = display.ratio;
                self.quota_vip = quota.vip;
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
