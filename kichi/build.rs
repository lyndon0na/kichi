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
    // 入口必须留在 qml/main.qml：main.rs 按这个 qrc 路径加载。
    "qml/main.qml",
    // 全局基础：主题、尺寸、页面索引、剪贴板
    "qml/core/ClipboardHelper.qml",
    "qml/core/KTheme.qml",
    "qml/core/PageIndex.qml",
    "qml/core/UiMetrics.qml",
    // 通用控件（K* 与小组件）
    "qml/components/DialogFooter.qml",
    "qml/components/ElidedLabel.qml",
    "qml/components/IndexBuildStatus.qml",
    "qml/components/KButton.qml",
    "qml/components/KColorSwatchRow.qml",
    "qml/components/KComboBox.qml",
    "qml/components/KDialog.qml",
    "qml/components/KHeaderCell.qml",
    "qml/components/KIconButton.qml",
    "qml/components/KLoadingSpinner.qml",
    "qml/components/KMediaToolTip.qml",
    "qml/components/KPlayingBars.qml",
    "qml/components/KProgressBar.qml",
    "qml/components/KRowChrome.qml",
    "qml/components/KScrollBar.qml",
    "qml/components/KScrollMemory.qml",
    "qml/components/KSectionTitle.qml",
    "qml/components/KSettingsGroup.qml",
    "qml/components/KSettingsRow.qml",
    "qml/components/KStatusBadge.qml",
    "qml/components/KStepper.qml",
    "qml/components/KTableHeader.qml",
    "qml/components/KTextField.qml",
    "qml/components/LoadingSkeleton.qml",
    "qml/components/LoadingSkeletonGate.qml",
    "qml/components/NavIcon.qml",
    "qml/components/ScrollFirstClickArea.qml",
    "qml/components/SearchEmptyState.qml",
    "qml/components/SearchNavigationOverlay.qml",
    "qml/components/SearchStatusBar.qml",
    "qml/components/VideoLoadingOverlay.qml",
    // 主窗口外壳：侧栏、账号卡片
    "qml/shell/AccountCard.qml",
    "qml/shell/QuotaIndicator.qml",
    "qml/shell/SideBar.qml",
    // 登录与退出
    "qml/auth/LoginPage.qml",
    "qml/auth/SignOutDialog.qml",
    // 文件浏览
    "qml/files/ContentGridView.qml",
    "qml/files/ContentViewToggle.qml",
    "qml/files/EntryRows.qml",
    "qml/files/FileActionDialogs.qml",
    "qml/files/FileActionLocks.qml",
    "qml/files/FileGridItem.qml",
    "qml/files/FileIcon.qml",
    "qml/files/FileListDensity.qml",
    "qml/files/FileListHeader.qml",
    "qml/files/FileNavigationState.qml",
    "qml/files/FilePageInteraction.qml",
    "qml/files/FilePropertiesDialog.qml",
    "qml/files/FileRow.qml",
    "qml/files/FileSelection.qml",
    "qml/files/FileSortMenu.qml",
    "qml/files/FileThumbnail.qml",
    "qml/files/ImagePreviewOverlay.qml",
    "qml/files/FilesPage.qml",
    "qml/files/FilesPageHeaderActions.qml",
    "qml/files/FilesPageMenu.qml",
    "qml/files/FilesPagePlayMenu.qml",
    "qml/files/FilesPageSearchBar.qml",
    "qml/files/FilesPageUploadDropArea.qml",
    // 资料库与播放历史
    "qml/library/LibraryContextMenu.qml",
    "qml/library/LibraryDateSection.qml",
    "qml/library/LibraryGridCard.qml",
    "qml/library/LibraryGroupedGridView.qml",
    "qml/library/LibraryGroupedListView.qml",
    "qml/library/LibraryHeaderControls.qml",
    "qml/library/LibraryPage.qml",
    "qml/library/LibraryRow.qml",
    "qml/library/LibrarySectionView.qml",
    "qml/library/LibraryTableHeader.qml",
    "qml/library/PlaybackHistoryPage.qml",
    "qml/library/RecentSavedPage.qml",
    "qml/library/StarredPage.qml",
    // 设置窗口
    "qml/settings/SettingsAbout.qml",
    "qml/settings/SettingsAccount.qml",
    "qml/settings/SettingsDialog.qml",
    "qml/settings/SettingsGeneral.qml",
    "qml/settings/SettingsOutline.qml",
    "qml/settings/SettingsPage.qml",
    "qml/settings/SettingsPlayback.qml",
    "qml/settings/SettingsPlaybackLanguage.qml",
    "qml/settings/SettingsTransfers.qml",
    // 内嵌播放窗口
    "qml/playback/PlayToast.qml",
    "qml/playback/PlaybackPopover.qml",
    "qml/playback/PlaybackPopoverRow.qml",
    "qml/playback/PlaybackQueuePanel.qml",
    "qml/playback/PlaybackSpeedPanel.qml",
    "qml/playback/PlaybackSubtitlePanel.qml",
    "qml/playback/PlaybackTrackMenu.qml",
    "qml/playback/PlaybackWindow.qml",
    // 离线任务与任务中心
    "qml/tasks/MagnetFileRow.qml",
    "qml/tasks/MagnetFileTree.qml",
    "qml/tasks/MagnetFilesDialog.qml",
    "qml/tasks/NewTaskDialog.qml",
    "qml/tasks/TaskCenterPage.qml",
    "qml/tasks/TaskCenterTabs.qml",
    "qml/tasks/TaskContextMenu.qml",
    "qml/tasks/TaskFolderPickerDialog.qml",
    "qml/tasks/TaskHeaderControls.qml",
    "qml/tasks/TaskRow.qml",
    "qml/tasks/TaskTableHeader.qml",
    "qml/tasks/TasksPage.qml",
    // 传输列表
    "qml/transfers/TransferContextMenu.qml",
    "qml/transfers/TransferHeaderControls.qml",
    "qml/transfers/TransferMainRow.qml",
    "qml/transfers/TransferStickyRow.qml",
    "qml/transfers/TransferTableDensity.qml",
    "qml/transfers/TransferTableHeader.qml",
    "qml/transfers/TransfersPage.qml",
    // 回收站
    "qml/trash/TrashActionDialogs.qml",
    "qml/trash/TrashHeaderControls.qml",
    "qml/trash/TrashPage.qml",
    "qml/trash/TrashRow.qml",
    "qml/trash/TrashTableHeader.qml",
    // 分享
    "qml/shares/ShareContextMenu.qml",
    "qml/shares/ShareDialog.qml",
    "qml/shares/ShareHeaderControls.qml",
    "qml/shares/ShareResultPane.qml",
    "qml/shares/ShareRevokeDialog.qml",
    "qml/shares/ShareRow.qml",
    "qml/shares/ShareTableHeader.qml",
    "qml/shares/SharesPage.qml",
    // 跨页面的选择对话框
    "qml/dialogs/FolderPickerListView.qml",
    "qml/dialogs/FolderPickerNavigation.qml",
    "qml/dialogs/MovePickerDialog.qml",
];

// 手写 C++：头文件交给 moc + qmltyperegistrar，
// 因此 MpvVideo 会并进 io.kichi 模块，QML 直接 import io.kichi 即可。
const CPP_FILES: &[&str] = &[
    "cpp/mpv_video_item.h",
    "cpp/mpv_video_item.cpp",
    "cpp/wayland_video_surface.h",
    "cpp/wayland_video_surface.cpp",
    "cpp/protocol/color-management-v1-protocol.cpp",
    "cpp/protocol/content-type-v1-protocol.cpp",
    "cpp/protocol/linux-dmabuf-v1-protocol.cpp",
    "cpp/protocol/presentation-time-protocol.cpp",
    "cpp/protocol/viewporter-protocol.cpp",
    "cpp/app_platform.h",
    "cpp/app_platform.cpp",
];

// 纯 C 头文件不参与 moc 编译，只登记依赖。
const CPP_HEADERS: &[&str] = &[
    "cpp/embed_ffi.h",
    "cpp/protocol/color-management-v1-client-protocol.h",
    "cpp/protocol/content-type-v1-client-protocol.h",
    "cpp/protocol/linux-dmabuf-v1-client-protocol.h",
    "cpp/protocol/presentation-time-client-protocol.h",
    "cpp/protocol/viewporter-client-protocol.h",
];

fn main() {
    require_libmpv();

    // Wayland 原生 surface：compositor / dmabuf / color-management。
    println!("cargo:rustc-link-lib=wayland-client");

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
