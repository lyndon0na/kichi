import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 传输任务页标题行右侧的全局操作。
RowLayout {
    id: root

    property var target: null

    readonly property int activeCount: target ? target.activeCount : 0
    readonly property int queuedCount: target ? target.queuedCount : 0
    readonly property int pausedCount: target ? target.pausedCount : 0
    readonly property bool hasFinished: target ? target.hasFinished : false
    readonly property bool hasRetryable: target ? target.hasRetryable : false
    readonly property bool backendReady: target !== null && target.backend !== null

    readonly property KTheme theme: KTheme { parent: root }

    spacing: theme.gapSm

    Item {
        Layout.fillWidth: true
    }

    KButton {
        objectName: "retryFailedButton"
        Layout.alignment: Qt.AlignVCenter
        visible: root.hasRetryable
        text: qsTr("重试失败项")
        variant: "secondary"
        implicitHeight: 32
        enabled: root.backendReady && root.hasRetryable
        onClicked: {
            if (root.target)
                root.target.retryFailedTransfers()
        }
    }

    KButton {
        objectName: "clearFinishedButton"
        Layout.alignment: Qt.AlignVCenter
        visible: root.hasFinished
        text: qsTr("清空已完成")
        variant: "ghost"
        implicitHeight: 32
        enabled: root.backendReady && root.hasFinished
        onClicked: root.target.confirmClear()
    }

    KButton {
        objectName: "cancelAllTransfersButton"
        Layout.alignment: Qt.AlignVCenter
        text: qsTr("取消全部")
        variant: "dangerOutline"
        implicitHeight: 32
        enabled: root.backendReady
                 && (root.activeCount > 0 || root.pausedCount > 0)
        onClicked: {
            if (root.target)
                root.target.confirmCancelAll()
        }
    }
}
