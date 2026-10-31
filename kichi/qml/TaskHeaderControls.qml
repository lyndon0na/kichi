import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 离线任务页标题行右侧的操作。
RowLayout {
    id: root

    property var target: null

    readonly property bool loading: target ? target.tasksLoading : false

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
        objectName: "tasksRefreshButton"
        Layout.alignment: Qt.AlignVCenter
        text: qsTr("刷新")
        iconName: "refresh"
        iconSize: 14
        busy: root.loading
        variant: "ghost"
        enabled: root.target !== null && !root.loading
        onClicked: {
            if (root.target)
                root.target.refresh()
        }
    }
}
