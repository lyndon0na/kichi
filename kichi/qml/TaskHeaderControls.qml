import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 离线任务页标题行右侧的状态与操作，和分享页使用同一套层级。
RowLayout {
    id: root

    property var target: null

    readonly property int activeCount: target ? target.activeCount : 0
    readonly property int taskCount: target ? target.taskCount : 0
    readonly property bool loading: target ? target.tasksLoading : false

    readonly property KTheme theme: KTheme { parent: root }

    spacing: theme.gapSm

    Label {
        objectName: "tasksHeaderStatus"
        Layout.fillWidth: true
        Layout.alignment: Qt.AlignVCenter
        text: root.activeCount > 0
              ? qsTr("%1 条任务进行中 · 共 %2 条").arg(root.activeCount).arg(root.taskCount)
              : root.taskCount > 0
                ? qsTr("共 %1 条离线任务").arg(root.taskCount) : ""
        color: theme.textMuted
        font.pixelSize: theme.fontMeta
        elide: Text.ElideRight
    }

    KButton {
        objectName: "newTaskButton"
        Layout.alignment: Qt.AlignVCenter
        text: qsTr("新任务")
        iconName: "plus"
        variant: "primary"
        onClicked: {
            if (root.target)
                root.target.openNewTaskDialog()
        }
    }

    KButton {
        objectName: "tasksRefreshButton"
        Layout.alignment: Qt.AlignVCenter
        text: qsTr("刷新")
        iconName: "refresh"
        variant: "ghost"
        enabled: root.target !== null && !root.loading
        onClicked: {
            if (root.target)
                root.target.refresh()
        }
    }
}
