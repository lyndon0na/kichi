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

    // GridView 在右 margin 与 cellWidth 同一轮变化时可能沿用旧的列数。
    // 轻推一次 cellWidth 再恢复绑定，强制它重新计算每一行的列数。
    property real reflowNudge: 0
    onScrollBarVisibleChanged: {
        root.reflowNudge = 0.01
        Qt.callLater(function () {
            if (root)
                root.reflowNudge = 0
        })
    }

    property alias view: gridView
    property alias contentY: gridView.contentY
    property alias contentHeight: gridView.contentHeight
    property alias currentIndex: gridView.currentIndex
    property alias count: gridView.count

    function forceActiveFocus() {
        gridView.forceActiveFocus()
    }

    function cancelFlick() {
        gridView.cancelFlick()
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

    /// 视口坐标 (x, y) 是否落在某张卡片本体上，命中返回下标，否则 -1。
    ///
    /// `GridView.indexAt` 是按 **cell** 命中的：卡片之间 14 / 18px 的缝隙也算在
    /// 这一格里（docs/pitfalls.md 第 44 条）。直接拿它判「有没有点在卡片上」，
    /// 缝隙会被当成卡片，右键就永远弹条目菜单。所以再用卡片自己的矩形判一次，
    /// 缝隙按空白处理。
    function hitItemIndex(x, y) {
        const index = gridView.indexAt(x, y + gridView.contentY)
        if (index < 0)
            return -1
        const item = gridView.itemAtIndex(index)
        if (!item)
            return -1
        // 委托的 x / y 不含 leftMargin（实测 leftMargin 12 时首列委托 x 仍是 0），
        // 不要自己加减 margin：把视口坐标映射进卡片自己的坐标系再判矩形。
        const local = gridView.mapToItem(item, x, y)
        return local.x >= 0 && local.y >= 0
               && local.x < item.width && local.y < item.height ? index : -1
    }

    GridView {
        id: gridView

        objectName: root.viewObjectName
        readonly property bool scrollBarVisible: root.scrollBarVisible
        anchors.fill: parent
        visible: root.visible
        model: root.model
        delegate: root.delegate
        cellWidth: root.cellSlotWidth + root.reflowNudge
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

        // 卡片自己的 MouseArea 会接走落在卡片上的右键（弹条目菜单），
        // 这里只接没落在卡片本体上的位置：缝隙、最后一行下方、左右留白。
        TapHandler {
            acceptedButtons: Qt.RightButton
            onTapped: function (point) {
                // TapHandler 的 position 以内容区为原点（扣掉 leftMargin、随 contentY 走），
                // 直接当视口坐标用会让命中判断和弹窗位置一起偏移；统一换算回网格坐标。
                // QML 侧没有 mapFromScene，只能从 scenePosition 经 null 项映射。
                const viewport = gridView.mapFromItem(
                    null, point.scenePosition.x, point.scenePosition.y)
                if (root.hitItemIndex(viewport.x, viewport.y) >= 0)
                    return
                root.blankContextMenuRequested(Qt.point(viewport.x, viewport.y))
            }
        }
    }

    ScrollFirstClickArea {
        id: firstClickArea

        objectName: "gridFirstClickArea"
        view: gridView
        scrollBarSlot: root.scrollBarSlot
        hitTest: function (x, y) {
            return root.hitItemIndex(x, y)
        }

        onClickedAt: function (index, modifiers, position) {
            const item = gridView.itemAtIndex(index)
            if (!item)
                return
            gridView.forceActiveFocus()
            firstClickArea.invokeFirstClick(item, index, modifiers,
                                            position.x, position.y)
        }
    }
}
