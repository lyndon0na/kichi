import QtQuick
import QtQuick.Controls

// 文件类内容页共用的网格容器。
// 页面只提供 model、密度和委托；列数、卡片尺寸、滚动条车道在这里统一计算。
Item {
    id: root

    property var model: []
    property var density: null
    property var metrics: null
    /// 是否固定为卡片底部的一行说明预留高度。
    property bool reserveDetail: false
    property string viewObjectName: "contentGrid"
    property Component delegate: null

    signal keyPressed(var event)
    signal blankContextMenuRequested(point position)

    readonly property int sideMargin: root.metrics ? root.metrics.pageMargin : 12
    readonly property int rightMargin: root.sideMargin
                                        + (root.metrics ? root.metrics.scrollBarSlot : 10)
    readonly property real availableWidth: Math.max(
        0, root.width - root.sideMargin - root.rightMargin)
    readonly property real minItemWidth: root.density
                                        ? root.density.gridMinItemWidth : 160
    readonly property int columnGap: root.density ? root.density.gridColumnGap : 14
    readonly property int rowGap: root.density ? root.density.gridRowGap : 18
    readonly property int columns: Math.max(1, Math.floor(
        (root.availableWidth + root.columnGap)
        / (root.minItemWidth + root.columnGap)))
    readonly property real cellSlotWidth: root.columns > 0
        ? Math.max(1, Math.floor(root.availableWidth / root.columns)) : 0
    readonly property real itemWidth: Math.max(1, root.cellSlotWidth - root.columnGap)
    readonly property real itemHeight: root.density
        ? root.density.gridItemHeight(root.itemWidth, root.reserveDetail) : 1
    readonly property real cellHeight: root.itemHeight + root.rowGap

    property alias view: gridView
    property alias contentY: gridView.contentY
    property alias contentHeight: gridView.contentHeight
    property alias currentIndex: gridView.currentIndex
    property alias count: gridView.count

    function forceActiveFocus() {
        gridView.forceActiveFocus()
    }

    function itemAtIndex(index) {
        return gridView.itemAtIndex(index)
    }

    function positionViewAtIndex(index, mode) {
        gridView.positionViewAtIndex(index, mode)
    }

    function indexAt(x, y) {
        return gridView.indexAt(x, y)
    }

    GridView {
        id: gridView

        objectName: root.viewObjectName
        anchors.fill: parent
        visible: root.visible
        model: root.model
        delegate: root.delegate
        cellWidth: root.cellSlotWidth
        cellHeight: root.cellHeight
        leftMargin: root.sideMargin
        rightMargin: root.rightMargin
        topMargin: 0
        bottomMargin: 0
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        ScrollBar.vertical: KScrollBar {}
        focus: true

        Keys.onPressed: function (event) {
            root.keyPressed(event)
        }

        onVisibleChanged: {
            if (visible)
                forceActiveFocus()
        }

        TapHandler {
            acceptedButtons: Qt.RightButton
            onTapped: function (point) {
                const x = point.position.x
                const y = point.position.y
                if (gridView.indexAt(x, y + gridView.contentY) >= 0)
                    return
                root.blankContextMenuRequested(Qt.point(x, y))
            }
        }
    }
}
