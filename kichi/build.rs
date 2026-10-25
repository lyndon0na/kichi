//! cxx-qt 构建：把 QML 模块与桥接对象编进二进制。
use cxx_qt_build::{CxxQtBuilder, QmlModule};

fn main() {
    CxxQtBuilder::new_qml_module(QmlModule::new("io.kichi").qml_file("qml/main.qml"))
        .files(["src/cxxqt_object.rs"])
        .build();
}
