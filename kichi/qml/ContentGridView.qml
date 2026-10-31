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
    readonly property int scrollBarSlot: root.metrics ? root.metrics.scrollBarSlot : 10
    readonly property real minItemWidth: root.density
                                        ? root.density.gridMinItemWidth : 160
    readonly property int columnGap: root.density ? root.density.gridColumnGap : 14
    readonly property int rowGap: root.density ? root.density.gridRowGap : 18
    // 末列 cell 尾部自带 columnGap，右 margin 先扣掉这一段。
    readonly property int noScrollBarRightMargin: Math.max(
        0, root.sideMargin - root.columnGap)
    readonly property int scrollBarRightMargin: Math.max(
        0, root.sideMargin + root.scrollBarSlot - root.columnGap)
    // 列数按有滚动条时的最窄可用宽度算，避免有无滚动条时列数来回变化。
    readonly property real layoutAvailableWidth: Math.max(
        0, root.width - root.sideMargin - root.scrollBarRightMargin)
    readonly property int columns: Math.max(1, Math.floor(
        (root.layoutAvailableWidth + root.columnGap)
        / (root.minItemWidth + root.columnGap)))
    readonly property real noScrollBarAvailableWidth: Math.max(
        0, root.width - root.sideMargin - root.noScrollBarRightMargin)
    readonly property real noScrollBarCellSlotWidth: Math.max(
        1, Math.floor(Math.max(0, root.noScrollBarAvailableWidth - 1)
                      / root.columns))
    readonly property real noScrollBarRemainder: Math.max(
        0, root.noScrollBarAvailableWidth
           - root.columns * root.noScrollBarCellSlotWidth)
    // 无滚动条时让卡片吸收末列尾间距和取整余量，使左右边距严格一致。
    readonly property real noScrollBarItemWidth: Math.max(
        1, root.noScrollBarCellSlotWidth - root.sideMargin
           + root.noScrollBarRemainder)
    readonly property real noScrollBarItemHeight: root.density
        ? root.density.gridItemHeight(root.noScrollBarItemWidth, root.reserveDetail) : 1
    readonly property int projectedRows: Math.ceil(gridView.count / root.columns)
    readonly property bool scrollBarVisible: root.projectedRows
                                             * (root.noScrollBarItemHeight + root.rowGap)
                                             > root.height
    readonly property int rightMargin: root.scrollBarVisible
                                       ? root.scrollBarRightMargin
                                       : root.noScrollBarRightMargin
    readonly property real availableWidth: Math.max(
        0, root.width - root.sideMargin - root.rightMargin)
    readonly property real cellSlotWidth: root.scrollBarVisible
        ? Math.max(1, Math.floor(root.availableWidth / root.columns))
        : root.noScrollBarCellSlotWidth
    readonly property real itemWidth: root.scrollBarVisible
        ? Math.max(1, root.cellSlotWidth - root.columnGap)
        : root.noScrollBarItemWidth
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
        readonly property bool scrollBarVisible: root.scrollBarVisible
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
