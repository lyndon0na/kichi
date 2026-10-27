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
    let mut app = QGuiApplication::new();
    let mut engine = QQmlApplicationEngine::new();

    if let Some(engine) = engine.as_mut() {
        engine.load(&QUrl::from("qrc:/qt/qml/io/kichi/qml/main.qml"));
    }

    if let Some(app) = app.as_mut() {
        app.exec();
    }
}
