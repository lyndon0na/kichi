import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 传输任务页标题行右侧的状态与全局操作。
RowLayout {
    id: root

    property var target: null

    readonly property var activeGroup: target ? target.activeGroup : null
    readonly property int activeCount: target ? target.activeCount : 0
    readonly property int queuedCount: target ? target.queuedCount : 0
    readonly property bool backendReady: target !== null && target.backend !== null

    readonly property KTheme theme: KTheme { parent: root }

    spacing: theme.gapSm

    Label {
        objectName: "transferHeaderStatus"
        Layout.fillWidth: true
        Layout.alignment: Qt.AlignVCenter
        text: root.target ? root.target.statusText : ""
        color: root.activeGroup !== null || root.queuedCount > 0
               ? theme.textSecondary : theme.textMuted
        font.pixelSize: theme.fontMeta
        elide: Text.ElideRight
    }

    KButton {
        objectName: "openDownloadDirButton"
        Layout.alignment: Qt.AlignVCenter
        text: qsTr("打开下载目录")
        variant: "secondary"
        implicitHeight: 32
        enabled: root.backendReady
        onClicked: root.target.backend.openDownloadDir()
    }

    KButton {
        objectName: "cancelAllTransfersButton"
        Layout.alignment: Qt.AlignVCenter
        text: qsTr("取消全部")
        variant: "secondary"
        implicitHeight: 32
        enabled: root.backendReady && root.activeCount > 0
        onClicked: root.target.backend.cancelDownloads()
    }
}
