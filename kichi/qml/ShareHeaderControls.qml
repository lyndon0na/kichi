import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 分享页标题行右侧的状态与操作，避免在表格上方再占一整行。
RowLayout {
    id: root

    property var target: null

    readonly property int selectedCount: target ? target.selectedCount : 0
    readonly property int shareCount: target ? target.shareCount : 0
    readonly property bool actionRunning: target ? target.actionRunning : false
    readonly property bool loading: target ? target.loading : false
    readonly property string copyNotice: target ? target.copyNotice : ""

    readonly property KTheme theme: KTheme { parent: root }

    spacing: theme.gapSm

    ElidedLabel {
        objectName: "shareHeaderStatus"
        Layout.fillWidth: true
        Layout.alignment: Qt.AlignVCenter
        text: root.copyNotice.length > 0
              ? root.copyNotice
              : root.selectedCount > 0
                ? qsTr("已选择 %1 个分享").arg(root.selectedCount)
                : root.shareCount > 0
                  ? qsTr("共 %1 个分享链接").arg(root.shareCount) : ""
        color: root.copyNotice.length > 0 ? theme.success : theme.textMuted
        font.pixelSize: theme.fontMeta
        elide: Text.ElideRight
    }

    KButton {
        objectName: "revokeSelectedShares"
        Layout.alignment: Qt.AlignVCenter
        text: root.selectedCount > 1
              ? qsTr("取消分享（%1）").arg(root.selectedCount)
              : qsTr("取消分享")
        variant: "dangerOutline"
        enabled: root.target !== null && root.selectedCount > 0 && !root.actionRunning
        onClicked: root.target.openRevoke(root.target.selectedIdsInOrder())
    }

    KButton {
        objectName: "refreshShares"
        Layout.alignment: Qt.AlignVCenter
        text: qsTr("刷新")
        iconName: "refresh"
        variant: "ghost"
        enabled: root.target !== null && !root.loading
        onClicked: root.target.refresh()
    }
}
