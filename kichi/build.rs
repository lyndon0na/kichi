//! cxx-qt 构建：把 QML 模块与桥接对象编进二进制。
//!
//! ⚠ QML 文件的变化 cargo 默认检测不到：QML 是经 `qml_file()` 读进 qrc 的，
//! 不走 Rust 依赖图。不加 `rerun-if-changed` 就会出现「改了 QML 但跑的还是
//! 旧界面」，排查起来极费时间（2026-10-26 S6 实测踩到）。因此下面逐个登记。
use cxx_qt_build::{CxxQtBuilder, QmlModule};

const QML_FILES: &[&str] = &[
    "qml/main.qml",
    "qml/PageIndex.qml",
    "qml/LoginPage.qml",
    "qml/SideBar.qml",
    "qml/AccountCard.qml",
    "qml/TransferStatusButton.qml",
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
    "qml/KProgressBar.qml",
    "qml/DialogFooter.qml",
    "qml/StandardDialogFooter.qml",
    "qml/UiMetrics.qml",
    "qml/KTheme.qml",
    "qml/FirstLoadGate.qml",
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
    "qml/SettingsPage.qml",
    "qml/SettingsDownloads.qml",
    "qml/SettingsAbout.qml",
    "qml/SettingsPlayback.qml",
    "qml/PlayToast.qml",
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
