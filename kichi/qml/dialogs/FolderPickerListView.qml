import QtQuick
import QtQuick.Controls
import "../components"
import "../core"

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

    // 加载期间保留旧列表；新模型就绪后再原子替换。进入子目录从顶部开始。
    function beginDirectoryLoad() {
        pressedIndex = -1
        view.cancelFlick()
    }

    /// 替换模型**之前**调用：先把列表拉回顶部。
    /// Qt 6.11 在列表停在旧底部时重灌更短的模型，会把 `originY` 留在旧值上
    /// （实测 384），新内容整体被推到视口下方；事后改 `contentY` 无法修正
    /// `originY`，所以必须抢在 `clear()` 之前回顶。
    function prepareDirectoryReplace() {
        view.positionViewAtBeginning()
        view.contentY = 0
        view.forceLayout()
    }

    /// `restoreY` 用于「上一级」和手动刷新：回到顶部之后再落到原位置。
    /// 进入子目录不传（或传 -1），保持从顶部显示。
    function finishDirectoryLoad(restoreY) {
        view.forceLayout()
        view.positionViewAtBeginning()
        view.contentY = 0
        view.forceLayout()
        if (restoreY !== undefined && restoreY > 0) {
            const maxY = Math.max(0, view.contentHeight - view.height)
            view.contentY = Math.min(restoreY, maxY)
        }
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
