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
        #[qproperty(bool, busy, READ, NOTIFY = changed)]
        #[qproperty(QString, account, READ, NOTIFY = changed)]
        #[qproperty(QString, quota_used_text, cxx_name = "quotaUsed", READ, NOTIFY = changed)]
        #[qproperty(QString, quota_limit_text, cxx_name = "quotaLimit", READ, NOTIFY = changed)]
        /// 容量占用比例 0..1；`-1` = 不限量 / 未知，QML 据此隐藏进度条
        #[qproperty(f64, quota_ratio, cxx_name = "quotaRatio", READ, NOTIFY = changed)]
        #[qproperty(bool, quota_vip, cxx_name = "quotaVip", READ, NOTIFY = changed)]
        #[qproperty(bool, captcha_required, cxx_name = "captchaRequired", READ, NOTIFY = changed)]
        #[qproperty(QVariantList, files, READ, NOTIFY = changed)]
        #[qproperty(QStringList, breadcrumb, READ, NOTIFY = changed)]
        #[qproperty(QVariantList, tasks, READ, NOTIFY = changed)]
        #[qproperty(QString, task_input, cxx_name = "taskInput", READ, WRITE, NOTIFY = changed)]
        #[qproperty(QVariantList, play_menu, cxx_name = "playMenu", READ, NOTIFY = changed)]
        #[qproperty(QString, play_title, cxx_name = "playTitle", READ, NOTIFY = changed)]
        #[qproperty(QString, preferred_quality, cxx_name = "preferredQuality", READ, NOTIFY = changed)]
        #[qproperty(QString, player_command, cxx_name = "playerCommand", READ, NOTIFY = changed)]
        #[qproperty(QString, player_args, cxx_name = "playerArgs", READ, NOTIFY = changed)]
        #[qproperty(QString, player_default_command, cxx_name = "playerDefaultCommand", READ, NOTIFY = changed)]
        #[qproperty(QString, player_default_args, cxx_name = "playerDefaultArgs", READ, NOTIFY = changed)]
        #[qproperty(bool, notify_enabled, cxx_name = "notifyEnabled", READ, NOTIFY = changed)]
        #[qproperty(bool, close_to_tray, cxx_name = "closeToTray", READ, NOTIFY = changed)]
        #[qproperty(bool, tray_available, cxx_name = "trayAvailable", READ, NOTIFY = changed)]
        #[qproperty(i32, tray_show_requested, cxx_name = "trayShowRequested", READ, NOTIFY = changed)]
        #[qproperty(bool, quit_requested, cxx_name = "quitRequested", READ, NOTIFY = changed)]
        #[qproperty(QString, sort_field_text, cxx_name = "sortField", READ, NOTIFY = changed)]
        #[qproperty(bool, sort_ascending, cxx_name = "sortAscending", READ, NOTIFY = changed)]
        #[qproperty(i32, window_width, cxx_name = "windowWidth", READ, NOTIFY = changed)]
        #[qproperty(i32, window_height, cxx_name = "windowHeight", READ, NOTIFY = changed)]
        #[qproperty(bool, window_maximized, cxx_name = "windowMaximized", READ, NOTIFY = changed)]
        #[qproperty(QString, settings_path, cxx_name = "settingsPath", READ, NOTIFY = changed)]
        #[qproperty(QString, storage_backend, cxx_name = "storageBackend", READ, NOTIFY = changed)]
        #[qproperty(QString, download_current, cxx_name = "downloadCurrent", READ, NOTIFY = changed)]
        #[qproperty(bool, download_running, cxx_name = "downloadRunning", READ, NOTIFY = changed)]
        #[qproperty(i32, download_remaining, cxx_name = "downloadRemaining", READ, NOTIFY = changed)]
        #[qproperty(i32, download_done, cxx_name = "downloadDone", READ, NOTIFY = changed)]
        #[qproperty(i32, download_failed, cxx_name = "downloadFailed", READ, NOTIFY = changed)]
        #[qproperty(QString, download_dir, cxx_name = "downloadDir", READ, NOTIFY = changed)]
        #[qproperty(bool, file_action_running, cxx_name = "fileActionRunning", READ, NOTIFY = changed)]
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

        /// 一次文件操作（改名 / 新建 / 删除）结束：动作、成功与否、给用户看的提示
        #[qsignal]
        #[cxx_name = "fileActionDone"]
        fn file_action_done(self: Pin<&mut Self>, action: QString, ok: bool, message: QString);

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

        #[qinvokable]
        #[cxx_name = "pump"]
        fn pump(self: Pin<&mut Self>);

        #[qinvokable]
        #[cxx_name = "refreshFiles"]
        fn refresh_files(self: Pin<&mut Self>);

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

        #[qinvokable]
        #[cxx_name = "addTask"]
        fn add_task(self: Pin<&mut Self>, url: QString);

        #[qinvokable]
        #[cxx_name = "deleteTask"]
        fn delete_task(self: Pin<&mut Self>, id: QString);

        #[qinvokable]
        #[cxx_name = "requestPlay"]
        fn request_play(self: Pin<&mut Self>, id: QString);

        #[qinvokable]
        #[cxx_name = "playSelected"]
        fn play_selected(self: Pin<&mut Self>, index: i32);

        #[qinvokable]
        #[cxx_name = "cancelPlay"]
        fn cancel_play(self: Pin<&mut Self>);

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
        #[cxx_name = "resetPlayer"]
        fn reset_player(self: Pin<&mut Self>);

        #[qinvokable]
        #[cxx_name = "setNotifyEnabled"]
        fn set_notify_enabled(self: Pin<&mut Self>, enabled: bool);

        #[qinvokable]
        #[cxx_name = "setCloseToTray"]
        fn set_close_to_tray(self: Pin<&mut Self>, enabled: bool);

        #[qinvokable]
        #[cxx_name = "setSorting"]
        fn set_sorting(self: Pin<&mut Self>, field: QString, ascending: bool);

        #[qinvokable]
        #[cxx_name = "saveWindowGeometry"]
        fn save_window_geometry(self: Pin<&mut Self>, width: i32, height: i32, maximized: bool);

        /// 下载一个文件或整个文件夹（文件夹递归排队）
        #[qinvokable]
        #[cxx_name = "downloadEntry"]
        fn download_entry(self: Pin<&mut Self>, id: QString);

        #[qinvokable]
        #[cxx_name = "cancelDownloads"]
        fn cancel_downloads(self: Pin<&mut Self>);

        #[qinvokable]
        #[cxx_name = "openDownloadDir"]
        fn open_download_dir(self: &Self);

        #[qinvokable]
        #[cxx_name = "setDownloadDir"]
        fn set_download_dir(self: Pin<&mut Self>, path: QString);

        /// 给条目改名（成功后自动刷新当前目录）
        #[qinvokable]
        #[cxx_name = "renameEntry"]
        fn rename_entry(self: Pin<&mut Self>, id: QString, name: QString);

        /// 在当前目录下新建文件夹
        #[qinvokable]
        #[cxx_name = "createFolder"]
        fn create_folder(self: Pin<&mut Self>, name: QString);

        /// 把条目移入回收站（可恢复，界面上要二次确认）
        #[qinvokable]
        #[cxx_name = "trashEntry"]
        fn trash_entry(self: Pin<&mut Self>, id: QString);

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

        /// 搜索：揭示文件所在的文件夹（文件夹条目就是进入它自己）
        #[qinvokable]
        #[cxx_name = "searchRevealFolder"]
        fn search_reveal_folder(self: Pin<&mut Self>, id: QString, is_folder: bool);
    }
}

use cxx_qt_lib::{QString, QStringList, QVariant};

/// Qt 的 `QVariantList`（即 `QList<QVariant>` 的别名）。
///
/// cxx-qt-lib 只导出 `QList<QVariant>`，其 ExternType Id 是 `QList<QVariant>`；
/// cxx 桥接里的类型别名要求 Rust 类型 Id 与声明名一致，因此包一层
/// `#[repr(transparent)]` 让桥接可以直接使用 Qt 的类型名。
#[repr(transparent)]
#[derive(Default)]
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
}

mod actions;
mod backend;
mod downloads;
mod file_actions;
mod files;
mod notifications;
mod play;
mod queue;
mod search;
mod settings;
mod tasks;
mod tray;
mod tray_icon;

use backend::Backend;
use files::Crumb;

/// QObject `Kichi` 的 Rust 载体。
pub struct KichiRust {
    pub(crate) auth_state: QString,
    pub(crate) status_message: QString,
    pub(crate) busy: bool,
    pub(crate) captcha_required: bool,
    pub(crate) account: QString,
    pub(crate) quota_used_text: QString,
    pub(crate) quota_limit_text: QString,
    pub(crate) quota_ratio: f64,
    pub(crate) quota_vip: bool,
    /// 当前目录的列表项（QML 直接当 model 用）
    pub(crate) files: QVariantList,
    /// 面包屑显示名（根 → 当前）
    pub(crate) breadcrumb: QStringList,
    /// 当前目录的条目（openDir 查名字用，避免 QML 传来路不明的显示名）
    pub(crate) entries: Vec<kichi_core::FileEntry>,
    /// 目录栈；根目录 id 为空串
    pub(crate) crumbs: Vec<Crumb>,
    /// 有目录列表请求在途（防止重复点击）
    pub(crate) listing: bool,
    /// 目录请求代数：退出登录 / 重登后旧结果作废
    pub(crate) list_epoch: u64,
    /// 当前离线任务（QML 直接当 model 用）
    pub(crate) tasks: QVariantList,
    /// 当前任务的领域对象（通知判断、删除确认用）
    pub(crate) task_list: Vec<kichi_core::OfflineTask>,
    /// 任务页输入框内容（QML 双向绑定，提交成功后由 Rust 清空）
    pub(crate) task_input: QString,
    /// 有任务请求在途
    pub(crate) tasks_loading: bool,
    /// 任务请求代数：退出登录 / 重登后旧结果作废
    pub(crate) tasks_epoch: u64,
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
    /// 播放请求代数：退出登录 / 重登后旧结果作废
    pub(crate) play_epoch: u64,
    /// 持久化设置（启动时读一次，改动即落盘）
    pub(crate) settings: crate::settings::Settings,
    /// 清晰度偏好（"ask" / "original" / "transcode"，QML 展示用）
    pub(crate) preferred_quality: QString,
    /// 自定义播放器命令 / 参数（空串 = 用默认）
    pub(crate) player_command: QString,
    pub(crate) player_args: QString,
    /// 内置默认播放器命令 / 参数（设置页展示与「恢复默认」参考）
    pub(crate) player_default_command: QString,
    pub(crate) player_default_args: QString,
    pub(crate) notify_enabled: bool,
    pub(crate) close_to_tray: bool,
    /// 托盘是否真的起来了（失败时界面禁用「关闭到托盘」）
    pub(crate) tray_available: bool,
    /// 托盘要求显示窗口的计数（QML 监听变化后 show/raise）
    pub(crate) tray_show_requested: i32,
    /// 托盘要求退出
    pub(crate) quit_requested: bool,
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
    /// 下载进度文本（当前文件 · 已下 / 总量）
    pub(crate) download_current: QString,
    /// 是否有一批下载在跑（决定界面显示进度条与取消按钮）
    pub(crate) download_running: bool,
    pub(crate) download_remaining: i32,
    pub(crate) download_done: i32,
    pub(crate) download_failed: i32,
    /// 下载目录的展示值（空串 = 用默认，界面上显示实际路径）
    pub(crate) download_dir: QString,
    /// 有文件操作（改名 / 新建 / 删除）在途，界面上禁用相关入口
    pub(crate) file_action_running: bool,
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
            busy: false,
            captcha_required: false,
            account: QString::default(),
            quota_used_text: QString::default(),
            quota_limit_text: QString::default(),
            quota_ratio: -1.0,
            quota_vip: false,
            files: QVariantList::default(),
            breadcrumb,
            entries: Vec::new(),
            crumbs: vec![Crumb::root()],
            listing: false,
            list_epoch: 0,
            tasks: QVariantList::default(),
            task_list: Vec::new(),
            task_input: QString::default(),
            tasks_loading: false,
            tasks_epoch: 0,
            known_running: std::collections::HashMap::new(),
            notified: std::collections::HashSet::new(),
            play_menu: QVariantList::default(),
            play_title: QString::default(),
            play_id: String::new(),
            play_pending: false,
            play_epoch: 0,
            settings: crate::settings::Settings::default(),
            preferred_quality: QString::from("ask"),
            player_command: QString::default(),
            player_args: QString::default(),
            player_default_command: QString::from(crate::player::DEFAULT_PLAYER),
            player_default_args: QString::from(crate::player::DEFAULT_ARGS),
            notify_enabled: true,
            close_to_tray: false,
            tray_available: false,
            tray_show_requested: 0,
            quit_requested: false,
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
            download_current: QString::default(),
            download_running: false,
            download_remaining: 0,
            download_done: 0,
            download_failed: 0,
            download_dir: QString::default(),
            file_action_running: false,
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
            .cloned()
    }

    /// 构造时读设置：属性先落地，再让界面看到（QML 构造时属性已可读）。
    fn load_settings(&mut self) {
        let path = crate::settings::default_path();
        self.settings_path = QString::from(path.to_string_lossy().as_ref());
        let (settings, warning) = crate::settings::load(&path);
        self.settings = settings;
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
        self.player_command = QString::from(settings.player_command.as_str());
        self.player_args = QString::from(settings.player_args.as_str());
        self.notify_enabled = settings.notify_enabled;
        self.close_to_tray = settings.close_to_tray;
        self.sort_field = settings.sort_field;
        self.sort_field_text = QString::from(settings.sort_field.as_str());
        self.sort_ascending = settings.sort_ascending;
        self.window_width = settings.window_width;
        self.window_height = settings.window_height;
        self.window_maximized = settings.window_maximized;
        // 下载目录属性展示「实际生效的路径」，空串表示用默认
        self.download_dir = QString::from(settings.download_dir.as_str());
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
            self.busy = true;
            self.status_message = QString::from("正在恢复会话……");
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
