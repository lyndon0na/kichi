import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 文件页标题行右侧的目录操作。刷新固定在最后，保持最右位置。
RowLayout {
    id: root

    property var backend: null
    property var target: null

    readonly property bool searching: backend !== null && backend.searchActive
    readonly property bool loading: backend !== null && backend.filesLoading
    readonly property bool actionRunning: backend !== null
                                          && backend.fileActionRunning

    readonly property KTheme theme: KTheme { parent: root }

    spacing: theme.gapXs

    KButton {
        objectName: "filesNewFolderButton"
        Layout.alignment: Qt.AlignVCenter
        text: qsTr("新建文件夹")
        iconName: "plus"
        iconSize: 14
        variant: "secondary"
        implicitHeight: 34
        leftPadding: 8
        rightPadding: 8
        enabled: root.backend !== null && !root.searching && !root.loading
                 && !root.actionRunning
        showTextToolTip: false
        ToolTip.visible: hovered
        ToolTip.text: qsTr("在当前文件夹中新建文件夹")
        onClicked: {
            if (root.target)
                root.target.beginNewFolder()
        }
    }

    FilesPageViewToggle {
        Layout.alignment: Qt.AlignVCenter
        backend: root.backend
        onViewRequested: function (view) {
            if (root.target)
                root.target.setFileView(view)
        }
    }

    KButton {
        objectName: "filesRefreshButton"
        Layout.alignment: Qt.AlignVCenter
        text: qsTr("刷新")
        iconName: "refresh"
        iconSize: 14
        variant: "ghost"
        implicitHeight: 34
        leftPadding: 8
        rightPadding: 8
        enabled: root.backend !== null && !root.searching && !root.loading
                 && !root.actionRunning
        showTextToolTip: false
        ToolTip.visible: hovered
        ToolTip.text: qsTr("刷新当前文件夹")
        onClicked: {
            if (root.target)
                root.target.refreshFiles()
        }
    }
}
