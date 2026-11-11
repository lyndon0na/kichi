//! kichi GUI 入口：装载 QML 主界面。
//!
//! MOC 预处理会把 `QML_IMPORT_NAME` / `QML_IMPORT_MAJOR_VERSION` 编译进
//! 生成的 QML 类型注册表，因此 QML 里可以直接 `import io.kichi 1.0`
//! 实例化 `Kichi {}`（无需从 C++ 侧注入根对象）。

mod appearance;
mod cxxqt_object;
mod format;
mod instance;
mod platform;
mod player;
mod session;
mod settings;

use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QString, QUrl};

extern "C" {
    /// `cpp/app_platform.cpp`：关闭「最后一个窗口关掉就退出」。
    fn kichi_set_quit_on_last_window_closed(enabled: bool);
}

fn main() {
    // 单实例检查必须最先做：第二个进程连上已有实例的 socket 后直接退出，
    // 不能先跑缓存清理，更不能建第二个窗口。锁文件要活到进程结束。
    let _instance_guard = match instance::claim() {
        instance::Claim::Secondary => return,
        instance::Claim::Primary(instance) => Some(instance),
        instance::Claim::Unprotected => None,
    };

    // 控件风格：KDE 用 org.kde.desktop（Breeze 外观 + KDE 原生调色板）；
    // 其他桌面用 Fusion。Fusion 完全由 QPalette 驱动，才能接住 XDG Desktop
    // Portal 给的深色偏好和强调色；org.kde.desktop 的开关等私有控件固定读
    // Kirigami / kdeglobals，在 GNOME 下无法只靠调色板变暗。
    // 用户显式设了环境变量就尊重用户（调试 / 换风格时别被这里覆盖）。
    if std::env::var_os("QT_QUICK_CONTROLS_STYLE").is_none() {
        let style = if platform::kde_session() {
            "org.kde.desktop"
        } else {
            "Fusion"
        };
        std::env::set_var("QT_QUICK_CONTROLS_STYLE", style);
    }
    // 只有纯 QPalette 驱动的风格才需要外观注入；KDE 原生风格由 KDE 自己维护。
    let palette_driven =
        std::env::var("QT_QUICK_CONTROLS_STYLE").map_or(true, |style| style != "org.kde.desktop");

    // 上次崩溃可能留下字幕临时目录：只清 24 小时前的，避免误删另一个实例正在用的。
    player::subtitles::sweep_stale(std::time::Duration::from_secs(24 * 60 * 60));
    // 持久字幕缓存：删 30 天前的，并把总大小收到 100 MiB 以内。
    session::subtitle_cache::sweep();
    // 文件列表缩略图：删 30 天前的，并把总大小收到 64 MiB 以内。
    session::thumbnail_cache::sweep();
    // 账号头像：删 30 天前的，并把总大小收到 8 MiB 以内。
    session::avatar_cache::sweep();
    // 图片预览临时文件：上次崩溃可能留下的旧图，只清 24 小时前的。
    session::image_preview::sweep_stale(std::time::Duration::from_secs(24 * 60 * 60));

    let mut app = QGuiApplication::new();

    // 桌面文件名必须和 Wayland app_id 一致。KWin 在授权新的激活令牌时，
    // 如果请求方 app_id 与当前活动窗口的 desktopFileName 不同，会按
    // StartupNotify=true 播放「正在启动」光标动画——内嵌播放窗口每次打开
    // 都会请求激活，桌面文件名空着就会每次都闪一次启动反馈。
    let desktop_file = if platform::flatpak_sandbox() {
        "io.github.lyndon0na.kichi"
    } else {
        "kichi"
    };
    QGuiApplication::set_desktop_file_name(&QString::from(desktop_file));

    // 播放窗口是独立的顶层窗口：主窗口缩到托盘后关掉播放窗口，不能连带结束整个
    // 进程。真退出由 main.qml 的关窗 / 托盘逻辑显式调用 Qt.quit()。
    unsafe { kichi_set_quit_on_last_window_closed(false) };

    // 先取一次系统外观再载入 QML：深色偏好 / 强调色直接成为窗口的默认
    // palette，避免首帧先画成浅色再闪一下。之后的变化由 Kichi::pump() 应用。
    if palette_driven {
        appearance::apply(appearance::start());
    }

    let mut engine = QQmlApplicationEngine::new();

    if let Some(engine) = engine.as_mut() {
        engine.load(&QUrl::from("qrc:/qt/qml/io/kichi/qml/main.qml"));
    }

    if let Some(app) = app.as_mut() {
        app.exec();
    }
}
