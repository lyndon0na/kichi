import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 回收站页标题行右侧的批量操作；仅选择后显示数量反馈。
RowLayout {
    id: root

    property var target: null

    readonly property int selectedCount: target ? target.selectedCount : 0
    readonly property bool busy: target ? target.busy : false
    readonly property bool backendReady: target !== null && target.backend !== null
    readonly property bool refreshing: backendReady && target.backend.trashLoading

    readonly property KTheme theme: KTheme { parent: root }

    spacing: theme.gapSm

    ElidedLabel {
        objectName: "trashSelectionSummary"
        Layout.fillWidth: true
        Layout.alignment: Qt.AlignVCenter
        text: root.selectedCount > 0
              ? qsTr("已选择 %1 项").arg(root.selectedCount) : ""
        color: root.selectedCount > 0 ? theme.textPrimary : theme.textMuted
        font.pixelSize: theme.fontMeta
        elide: Text.ElideRight
    }

    KButton {
        objectName: "trashRestoreButton"
        Layout.alignment: Qt.AlignVCenter
        text: qsTr("还原")
        iconName: "restore"
        variant: "primary"
        enabled: root.backendReady && root.selectedCount > 0 && !root.busy
        onClicked: root.target.restoreSelected()
    }

    KButton {
        objectName: "trashDeleteButton"
        Layout.alignment: Qt.AlignVCenter
        text: qsTr("彻底删除")
        iconName: "trash"
        variant: "dangerOutline"
        enabled: root.backendReady && root.selectedCount > 0 && !root.busy
        onClicked: root.target.deleteSelected()
    }

    ContentViewToggle {
        Layout.alignment: Qt.AlignVCenter
        backend: root.target ? root.target.backend : null
        onViewRequested: function (view) {
            if (root.target)
                root.target.setContentView(view)
        }
    }

    KButton {
        objectName: "trashRefreshButton"
        Layout.alignment: Qt.AlignVCenter
        text: root.refreshing ? qsTr("刷新中…") : qsTr("刷新")
        iconName: "refresh"
        variant: "ghost"
        enabled: root.backendReady && !root.busy
        onClicked: root.target.backend.refreshTrash()
    }
}
