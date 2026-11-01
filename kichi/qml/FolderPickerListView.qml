import QtQuick
import QtQuick.Controls

// 目录选择弹窗共用的列表。
//
// HACK(Qt 6.11): 滚轮会让 Flickable 进入短暂的惯性滚动，期间第一次点击被
// 当作“停止滚动”吞掉。外层 MouseArea 只在此时接管首击并转发给当前行；
// 不要安装 WheelHandler，滚轮和惯性必须留给 Flickable 原生处理。
Item {
    id: root

    property alias model: view.model
    property alias delegate: view.delegate
    property alias contentY: view.contentY
    readonly property real contentHeight: view.contentHeight
    readonly property bool moving: view.moving

    signal rowActivated(int index)

    clip: true
    property int pressedIndex: -1

    UiMetrics {
        id: metrics
    }

    function indexAt(x, y) {
        return view.indexAt(x, y)
    }

    function itemAtIndex(index) {
        return view.itemAtIndex(index)
    }

    // 加载期间保留旧列表；新模型就绪后再原子切换到新目录顶部。
    function beginDirectoryLoad() {
        pressedIndex = -1
        view.cancelFlick()
    }

    function finishDirectoryLoad() {
        view.forceLayout()
        view.positionViewAtBeginning()
        view.contentY = 0
        view.forceLayout()
    }

    ListView {
        id: view

        anchors.fill: parent
        spacing: 2
        boundsBehavior: Flickable.StopAtBounds
        ScrollBar.vertical: KScrollBar {}
    }

    MouseArea {
        x: 0
        y: 0
        width: Math.max(0, view.width - metrics.scrollBarSlot)
        height: view.height
        z: 1
        // 只在惯性滚动期间接管内容区；滚动条车道始终留给 KScrollBar。
        enabled: view.moving || pressed
        acceptedButtons: Qt.LeftButton

        onPressed: function (mouse) {
            pressedIndex = view.indexAt(mouse.x, mouse.y + view.contentY)
        }

        onClicked: function (mouse) {
            if (pressedIndex >= 0)
                root.rowActivated(pressedIndex)
            pressedIndex = -1
        }

        onCanceled: {
            pressedIndex = -1
        }

    }
}
