//! cxx-qt 构建：把 QML 模块与桥接对象编进二进制。
//!
//! ⚠ QML 文件的变化 cargo 默认检测不到：QML 是经 `qml_file()` 读进 qrc 的，
//! 不走 Rust 依赖图。不加 `rerun-if-changed` 就会出现「改了 QML 但跑的还是
//! 旧界面」，排查起来极费时间（2026-10-26 S6 实测踩到）。因此下面逐个登记。
use cxx_qt_build::{CxxQtBuilder, QmlModule};

/// libmpv2-sys 只靠 `pkg-config mpv` 找链接库；缺开发文件时它给出的英文报错
/// 很难和普通编译失败区分，这里先给一条能直接照做的中文提示。
fn require_libmpv() {
    let found = std::process::Command::new("pkg-config")
        .args(["--exists", "mpv"])
        .status()
        .is_ok_and(|status| status.success());
    assert!(
        found,
        "未找到 libmpv 开发文件（pkg-config 找不到 mpv）。\n\
         内嵌播放器需要 libmpv 的头文件与链接库，请先安装：\n\
         Fedora:  sudo dnf install mpv-devel\n\
         Arch:    sudo pacman -S mpv\n\
         Debian:  sudo apt install libmpv-dev\n\
         安装后重新运行 cargo build。"
    );
}

const QML_FILES: &[&str] = &[
    "qml/main.qml",
    "qml/PageIndex.qml",
    "qml/LoginPage.qml",
    "qml/SideBar.qml",
    "qml/AccountCard.qml",
    "qml/QuotaIndicator.qml",
    "qml/LibraryMenuButton.qml",
    "qml/NavIcon.qml",
    "qml/FileIcon.qml",
    "qml/FileThumbnail.qml",
    "qml/KLoadingSpinner.qml",
    "qml/KPlayingBars.qml",
    "qml/VideoLoadingOverlay.qml",
    "qml/FilesPage.qml",
    "qml/FilesPageUploadDropArea.qml",
    "qml/LibraryPage.qml",
    "qml/LibraryRow.qml",
    "qml/LibraryTableHeader.qml",
    "qml/LibraryDateSection.qml",
    "qml/LibraryGridCard.qml",
    "qml/LibraryGroupedGridView.qml",
    "qml/LibraryGroupedListView.qml",
    "qml/LibrarySectionView.qml",
    "qml/LibraryContextMenu.qml",
    "qml/LibraryHeaderControls.qml",
    "qml/RecentSavedPage.qml",
    "qml/PlaybackHistoryPage.qml",
    "qml/StarredPage.qml",
    "qml/FilesPageMenu.qml",
    "qml/FileSortMenu.qml",
    "qml/FilesPagePlayMenu.qml",
    "qml/FilesPageSearchBar.qml",
    "qml/FileActionDialogs.qml",
    "qml/ClipboardHelper.qml",
    "qml/ShareDialog.qml",
    "qml/ShareResultPane.qml",
    "qml/ShareContextMenu.qml",
    "qml/ShareRevokeDialog.qml",
    "qml/FileSelection.qml",
    "qml/FileActionLocks.qml",
    "qml/FilePageInteraction.qml",
    "qml/MovePickerDialog.qml",
    "qml/FolderPickerListView.qml",
    "qml/SearchStatusBar.qml",
    "qml/SearchEmptyState.qml",
    "qml/SearchNavigationOverlay.qml",
    "qml/FileListHeader.qml",
    "qml/FileListDensity.qml",
    "qml/EntryRows.qml",
    "qml/FileRow.qml",
    "qml/FileGridItem.qml",
    "qml/ContentGridView.qml",
    "qml/ScrollFirstClickArea.qml",
    "qml/ContentViewToggle.qml",
    "qml/FilesPageHeaderActions.qml",
    "qml/KScrollBar.qml",
    "qml/KScrollMemory.qml",
    "qml/KRowChrome.qml",
    "qml/KTableHeader.qml",
    "qml/KHeaderCell.qml",
    "qml/FolderPickerNavigation.qml",
    "qml/KProgressBar.qml",
    "qml/DialogFooter.qml",
    "qml/UiMetrics.qml",
    "qml/KTheme.qml",
    "qml/LoadingSkeletonGate.qml",
    "qml/LoadingSkeleton.qml",
    "qml/KButton.qml",
    "qml/KIconButton.qml",
    "qml/KTextField.qml",
    "qml/ElidedLabel.qml",
    "qml/KComboBox.qml",
    "qml/KStatusBadge.qml",
    "qml/KSectionTitle.qml",
    "qml/KSettingsGroup.qml",
    "qml/KSettingsRow.qml",
    "qml/KDialog.qml",
    "qml/NewTaskDialog.qml",
    "qml/MagnetFilesDialog.qml",
    "qml/MagnetFileTree.qml",
    "qml/MagnetFileRow.qml",
    "qml/TaskFolderPickerDialog.qml",
    "qml/TasksPage.qml",
    "qml/TaskHeaderControls.qml",
    "qml/TaskTableHeader.qml",
    "qml/TaskRow.qml",
    "qml/TaskContextMenu.qml",
    "qml/TaskCenterPage.qml",
    "qml/TaskCenterTabs.qml",
    "qml/TrashPage.qml",
    "qml/TrashRow.qml",
    "qml/TrashTableHeader.qml",
    "qml/TrashActionDialogs.qml",
    "qml/SharesPage.qml",
    "qml/ShareHeaderControls.qml",
    "qml/TransferHeaderControls.qml",
    "qml/TrashHeaderControls.qml",
    "qml/ShareTableHeader.qml",
    "qml/ShareRow.qml",
    "qml/TransfersPage.qml",
    "qml/TransferTableDensity.qml",
    "qml/TransferTableHeader.qml",
    "qml/TransferMainRow.qml",
    "qml/TransferStickyRow.qml",
    "qml/TransferContextMenu.qml",
    "qml/SettingsDialog.qml",
    "qml/SignOutDialog.qml",
    "qml/SettingsPage.qml",
    "qml/SettingsOutline.qml",
    "qml/SettingsTransfers.qml",
    "qml/SettingsGeneral.qml",
    "qml/SettingsAccount.qml",
    "qml/SettingsAbout.qml",
    "qml/SettingsPlayback.qml",
    "qml/PlayToast.qml",
    "qml/PlaybackWindow.qml",
    "qml/PlaybackTrackMenu.qml",
    "qml/PlaybackPopover.qml",
    "qml/PlaybackPopoverRow.qml",
    "qml/PlaybackSpeedPanel.qml",
    "qml/PlaybackSubtitlePanel.qml",
    "qml/KMediaToolTip.qml",
    "qml/KStepper.qml",
    "qml/KColorSwatchRow.qml",
];

// 手写 C++：头文件交给 moc + qmltyperegistrar，
// 因此 MpvVideo 会并进 io.kichi 模块，QML 直接 import io.kichi 即可。
const CPP_FILES: &[&str] = &[
    "cpp/mpv_video_item.h",
    "cpp/mpv_video_item.cpp",
    "cpp/app_platform.h",
    "cpp/app_platform.cpp",
];

// 纯 C 头文件不参与 moc 编译，只登记依赖。
const CPP_HEADERS: &[&str] = &["cpp/embed_ffi.h"];

fn main() {
    require_libmpv();

    // 手写 C++ 直接用 eglGetProcAddress 取扩展入口（Qt 不导出该符号）。
    println!("cargo:rustc-link-lib=EGL");

    // QML 与桥接源码任何改动都要重跑本构建脚本
    for file in QML_FILES {
        println!("cargo:rerun-if-changed={file}");
    }
    println!("cargo:rerun-if-changed=src/cxxqt_object.rs");
    for file in CPP_FILES.iter().chain(CPP_HEADERS) {
        println!("cargo:rerun-if-changed={file}");
    }

    // 手写 C++ 类型以 QQuickItem 为基类，qmllint 需要经模块依赖解析到 QtQuick。
    let mut module = QmlModule::new("io.kichi").depend("QtQuick");
    for file in QML_FILES {
        module = module.qml_file(*file);
    }

    let mut builder = CxxQtBuilder::new_qml_module(module)
        .files(["src/cxxqt_object.rs"])
        // QQuickItem 派生类需要链接 QtQuick；QML_ELEMENT 注册走 QtQml。
        .qt_module("Quick")
        .qt_module("Qml");
    for file in CPP_FILES {
        builder = builder.cpp_file(*file);
    }
    builder.build();
}
