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
        #[qproperty(QString, quota_summary, cxx_name = "quotaSummary", READ, NOTIFY = changed)]
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
        type Kichi = super::KichiRust;

        /// 统一变更通知：所有属性都挂这一个信号
        #[qsignal]
        fn changed(self: Pin<&mut Self>);

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
}

mod actions;
mod backend;
mod files;
mod notifications;
mod play;
mod settings;
mod tasks;
mod tray;

use backend::Backend;
use files::Crumb;

/// QObject `Kichi` 的 Rust 载体。
pub struct KichiRust {
    pub(crate) auth_state: QString,
    pub(crate) status_message: QString,
    pub(crate) busy: bool,
    pub(crate) captcha_required: bool,
    pub(crate) account: QString,
    pub(crate) quota_summary: QString,
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
    /// 播放菜单（QML 当 model 用：label / available / reason / original）
    pub(crate) play_menu: QVariantList,
    /// 菜单标题（文件名）
    pub(crate) play_title: QString,
    /// 菜单对应的档位（含直链；只在 Rust 侧流转，不暴露给 QML）
    pub(crate) pending_options: Vec<kichi_core::PlayOption>,
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
            quota_summary: QString::default(),
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
            pending_options: Vec::new(),
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
            captcha_url: String::new(),
            backend: Backend::new(crate::session::store_path()),
        };
        object.load_settings();
        object.apply_restored();
        object
    }
}

impl KichiRust {
    /// 按 id 找当前列表里的条目名（不看条目类型，调用方自己判断）。
    pub(crate) fn entry_name(&self, id: &str) -> Option<String> {
        self.entries
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| entry.name.clone())
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
