//! cxx-qt 构建：把 QML 模块与桥接对象编进二进制。
//!
//! ⚠ QML 文件的变化 cargo 默认检测不到：QML 是经 `qml_file()` 读进 qrc 的，
//! 不走 Rust 依赖图。不加 `rerun-if-changed` 就会出现「改了 QML 但跑的还是
//! 旧界面」，排查起来极费时间（2026-10-26 S6 实测踩到）。因此下面逐个登记。
use cxx_qt_build::{CxxQtBuilder, QmlModule};

const QML_FILES: &[&str] = &[
    "qml/main.qml",
    "qml/LoginPage.qml",
    "qml/SideBar.qml",
    "qml/NavIcon.qml",
    "qml/FilesPage.qml",
    "qml/FilesPageMenu.qml",
    "qml/FilesPagePlayMenu.qml",
    "qml/FilesPageSearchBar.qml",
    "qml/SearchResults.qml",
    "qml/FileListHeader.qml",
    "qml/FileListDensity.qml",
    "qml/FileRow.qml",
    "qml/KScrollBar.qml",
    "qml/KProgressBar.qml",
    "qml/UiMetrics.qml",
    "qml/TasksPage.qml",
    "qml/TransfersPage.qml",
    "qml/SettingsPage.qml",
    "qml/SettingsAbout.qml",
    "qml/SettingsPlayback.qml",
];

fn main() {
    // QML 与桥接源码任何改动都要重跑本构建脚本
    for file in QML_FILES {
        println!("cargo:rerun-if-changed={file}");
    }
    println!("cargo:rerun-if-changed=src/cxxqt_object.rs");

    let mut module = QmlModule::new("io.kichi");
    for file in QML_FILES {
        module = module.qml_file(*file);
    }

    CxxQtBuilder::new_qml_module(module)
        .files(["src/cxxqt_object.rs"])
        .build();
}
