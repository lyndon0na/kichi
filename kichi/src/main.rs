//! kichi GUI 入口：装载 QML 主界面。
//!
//! MOC 预处理会把 `QML_IMPORT_NAME` / `QML_IMPORT_MAJOR_VERSION` 编译进
//! 生成的 QML 类型注册表，因此 QML 里可以直接 `import io.kichi 1.0`
//! 实例化 `Kichi {}`（无需从 C++ 侧注入根对象）。

mod cxxqt_object;
mod format;
mod player;
mod session;
mod settings;

use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QUrl};

fn main() {
    // 控件风格：跟随 KDE 桌面（Breeze 图标 + 系统配色方案）。
    // 不指定时 Qt 用 Basic 风格——一套刻意不带装饰的裸控件，菜单 / 弹窗 / 输入框
    // 会显得和桌面其它应用不是一家人。
    // 用户显式设了环境变量就尊重用户（调试 / 换风格时别被这里覆盖）。
    if std::env::var_os("QT_QUICK_CONTROLS_STYLE").is_none() {
        std::env::set_var("QT_QUICK_CONTROLS_STYLE", "org.kde.desktop");
    }

    // 上次崩溃可能留下字幕临时目录：只清 24 小时前的，避免误删另一个实例正在用的。
    player::subtitles::sweep_stale(std::time::Duration::from_secs(24 * 60 * 60));
    // 持久字幕缓存：删 30 天前的，并把总大小收到 100 MiB 以内。
    session::subtitle_cache::sweep();
    // 文件列表缩略图：删 30 天前的，并把总大小收到 64 MiB 以内。
    session::thumbnail_cache::sweep();

    let mut app = QGuiApplication::new();
    let mut engine = QQmlApplicationEngine::new();

    if let Some(engine) = engine.as_mut() {
        engine.load(&QUrl::from("qrc:/qt/qml/io/kichi/qml/main.qml"));
    }

    if let Some(app) = app.as_mut() {
        app.exec();
    }
}
