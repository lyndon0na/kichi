import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 离线任务页标题行右侧的操作。
RowLayout {
    id: root

    property var target: null

    readonly property bool loading: target ? target.tasksLoading : false
    readonly property bool clearing: target ? target.tasksClearing === true : false
    readonly property int finishedCount: target ? target.finishedCount : 0
    readonly property bool backendReady: target !== null && target.backend !== null
    readonly property bool showClearFinished: root.finishedCount > 0 || root.clearing

    readonly property KTheme theme: KTheme { parent: root }

    spacing: theme.gapSm

    Item {
        Layout.fillWidth: true
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
        objectName: "clearFinishedTasksButton"
        Layout.alignment: Qt.AlignVCenter
        visible: root.showClearFinished
        text: qsTr("清理已结束任务")
        busy: root.clearing
        variant: "ghost"
        enabled: root.backendReady && root.finishedCount > 0 && !root.clearing
        onClicked: {
            if (root.target)
                root.target.confirmClearFinished()
        }
    }

    KButton {
        objectName: "tasksRefreshButton"
        Layout.alignment: Qt.AlignVCenter
        text: qsTr("刷新")
        iconName: "refresh"
        iconSize: 14
        busy: root.loading
        variant: "ghost"
        enabled: root.backendReady && !root.loading && !root.clearing
        onClicked: {
            if (root.target)
                root.target.refresh()
        }
    }
}
