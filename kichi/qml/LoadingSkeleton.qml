import QtQuick

// 数据页第一次读取时的内容骨架。
//
// 页面本身保留页头、表头和滚动条车道，只在内容区盖一层与当前列表 / 网格
// 同尺寸的占位块。颜色和动画都从 KTheme 取，避免维护第二套视觉变量。
Item {
    id: skeleton

    objectName: "pageLoadingSkeleton"
    clip: true
    visible: false

    property bool gridMode: false
    property var density: null
    property var metrics: null
    /// 当前网格容器；用于读取与真实卡片完全一致的列宽 / 卡片尺寸。
    property var gridView: null
    /// 列表右侧的列宽，顺序与表头一致。
    property var trailingColumnWidths: []
    /// 列表模式下给页内表头留出的高度；网格模式不受影响。
    property int listTopInset: 0

    readonly property KTheme theme: KTheme { parent: skeleton }

    property real pulse: 1

    property int contentPadding: skeleton.density
                                 ? skeleton.density.rowPadding : 12
    property int columnSpacing: skeleton.density
                                ? skeleton.density.columnSpacing : 12
    readonly property int scrollBarSlot: skeleton.metrics
                                         ? skeleton.metrics.scrollBarSlot : 10
    property int rowHeight: skeleton.density
                            ? skeleton.density.rowHeight : 48
    property int leadingWidth: skeleton.density
                               ? skeleton.density.thumbnailSlotWidth : 36
    property int leadingHeight: skeleton.density
                                ? skeleton.density.thumbnailSlotHeight : 24
    property int leadingRadius: skeleton.theme.radiusSmall
    readonly property real trailingWidth: {
        const widths = skeleton.trailingColumnWidths || []
        let total = 0
        for (let i = 0; i < widths.length; ++i)
            total += Math.max(0, Number(widths[i]) || 0)
        if (widths.length > 1)
            total += skeleton.columnSpacing * (widths.length - 1)
        return total
    }

    // —— 列表骨架：沿用真实表格的 48px 行高与列分区 ——
    Column {
        id: listSkeleton

        objectName: "listLoadingSkeleton"
        anchors.fill: parent
        anchors.topMargin: skeleton.listTopInset
        visible: !skeleton.gridMode

        Repeater {
            model: skeleton.visible
                   ? Math.max(1, Math.ceil(skeleton.height / skeleton.rowHeight) + 1)
                   : 0

            delegate: Item {
                width: listSkeleton.width
                height: skeleton.rowHeight

                Item {
                    id: rowContent

                    x: skeleton.contentPadding
                    width: Math.max(0, parent.width - skeleton.contentPadding * 2
                                    - skeleton.scrollBarSlot)
                    height: parent.height
                    opacity: skeleton.pulse

                    Rectangle {
                        visible: skeleton.leadingWidth > 0
                        width: skeleton.leadingWidth
                        height: skeleton.leadingHeight
                        radius: skeleton.leadingRadius
                        anchors.verticalCenter: parent.verticalCenter
                        color: skeleton.theme.surfaceRaised
                        border.width: 1
                        border.color: skeleton.theme.hairline
                    }

                    Row {
                        id: trailingColumns

                        anchors.right: parent.right
                        anchors.verticalCenter: parent.verticalCenter
                        spacing: skeleton.columnSpacing

                        Repeater {
                            model: skeleton.trailingColumnWidths || []

                            delegate: Item {
                                width: Math.max(0, Number(modelData) || 0)
                                height: 8

                                Rectangle {
                                    width: parent.width * 0.62
                                    height: 8
                                    radius: 2
                                    anchors.right: parent.right
                                    anchors.verticalCenter: parent.verticalCenter
                                    color: skeleton.theme.surfacePressed
                                }
                            }
                        }
                    }

                    Item {
                        id: nameSlot

                        x: skeleton.leadingWidth > 0
                           ? skeleton.leadingWidth + skeleton.columnSpacing : 0
                        width: Math.max(12, parent.width - x
                                        - (trailingColumns.width > 0
                                           ? trailingColumns.width + skeleton.columnSpacing : 0))
                        height: parent.height

                        Rectangle {
                            width: nameSlot.width * (0.54 + (index % 3) * 0.11)
                            height: 10
                            radius: 2
                            anchors.verticalCenter: parent.verticalCenter
                            color: skeleton.theme.surfacePressed
                        }
                    }
                }

                Rectangle {
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.bottom: parent.bottom
                    height: 1
                    color: skeleton.theme.divider
                }
            }
        }
    }

    // —— 网格骨架：直接读取网格容器的列数 / 卡片尺寸 ——
    Item {
        id: gridSkeleton

        objectName: "gridLoadingSkeleton"
        anchors.fill: parent
        visible: skeleton.gridMode
        clip: true

        readonly property int columns: skeleton.gridView
                                       ? Math.max(1, skeleton.gridView.columns) : 1
        readonly property real cellWidth: skeleton.gridView
                                          ? skeleton.gridView.cellSlotWidth
                                          : Math.max(1, skeleton.width
                                                     - 2 * skeleton.contentPadding)
        readonly property real leftMargin: skeleton.gridView
                                           ? skeleton.gridView.sideMargin
                                           : skeleton.contentPadding
        readonly property real rowGap: skeleton.gridView
                                       ? skeleton.gridView.rowGap : 18
        readonly property real itemWidth: skeleton.gridView
                                          ? skeleton.gridView.itemWidth
                                          : Math.max(1, skeleton.width
                                                     - 2 * skeleton.contentPadding)
        readonly property real itemHeight: skeleton.gridView
                                           ? skeleton.gridView.itemHeight : 160
        readonly property real cardPadding: skeleton.density
                                            ? skeleton.density.gridCardPadding : 5
        readonly property real cardSpacing: skeleton.density
                                            ? skeleton.density.gridCardSpacing : 7
        readonly property int nameLineHeight: skeleton.density
                                              ? skeleton.density.gridNameLineHeight : 19
        readonly property int nameBlockHeight: skeleton.density
                                               ? skeleton.density.gridNameBlockHeight : 38
        readonly property int detailLineHeight: skeleton.density
                                                ? skeleton.density.gridDetailLineHeight : 17
        readonly property real thumbnailWidth: Math.max(
            1, gridSkeleton.itemWidth - gridSkeleton.cardPadding * 2)
        readonly property real thumbnailHeight: Math.max(
            1, Math.ceil(gridSkeleton.thumbnailWidth * 2 / 3))
        readonly property int visibleRows: Math.max(
            1, Math.ceil(gridSkeleton.height
                         / Math.max(1, gridSkeleton.itemHeight + gridSkeleton.rowGap)))

        Repeater {
            model: skeleton.visible
                   ? Math.min(60, gridSkeleton.columns * (gridSkeleton.visibleRows + 1))
                   : 0

            delegate: Item {
                readonly property int column: index % gridSkeleton.columns
                readonly property int row: Math.floor(index / gridSkeleton.columns)

                x: gridSkeleton.leftMargin
                   + column * gridSkeleton.cellWidth
                y: row * (gridSkeleton.itemHeight + gridSkeleton.rowGap)
                width: gridSkeleton.itemWidth
                height: gridSkeleton.itemHeight
                opacity: skeleton.pulse

                Rectangle {
                    width: gridSkeleton.thumbnailWidth
                    height: Math.min(gridSkeleton.thumbnailHeight,
                                     Math.max(0, parent.height - gridSkeleton.cardPadding * 2
                                               - gridSkeleton.cardSpacing
                                               - gridSkeleton.nameBlockHeight
                                               - gridSkeleton.detailLineHeight))
                    x: gridSkeleton.cardPadding
                    y: gridSkeleton.cardPadding
                    radius: skeleton.density
                            ? skeleton.density.gridThumbnailRadius(parent.width) : 8
                    color: skeleton.theme.surfaceRaised
                    border.width: 1
                    border.color: skeleton.theme.hairline
                }

                Rectangle {
                    width: parent.width * 0.82
                    height: 10
                    x: gridSkeleton.cardPadding
                    y: gridSkeleton.cardPadding + gridSkeleton.thumbnailHeight
                       + gridSkeleton.cardSpacing + 3
                    radius: 2
                    color: skeleton.theme.surfacePressed
                }

                Rectangle {
                    width: parent.width * 0.58
                    height: 10
                    x: gridSkeleton.cardPadding
                    y: gridSkeleton.cardPadding + gridSkeleton.thumbnailHeight
                       + gridSkeleton.cardSpacing + gridSkeleton.nameLineHeight + 3
                    radius: 2
                    color: skeleton.theme.surfacePressed
                }

                Rectangle {
                    width: parent.width * 0.46
                    height: 8
                    x: gridSkeleton.cardPadding
                    y: gridSkeleton.cardPadding + gridSkeleton.thumbnailHeight
                       + gridSkeleton.cardSpacing + gridSkeleton.nameBlockHeight + 4
                    radius: 2
                    color: skeleton.theme.surfacePressed
                }
            }
        }
    }

    SequentialAnimation {
        objectName: "skeletonPulseAnimation"
        running: skeleton.visible && skeleton.theme.motionNormal > 0
        loops: Animation.Infinite

        NumberAnimation {
            target: skeleton
            property: "pulse"
            from: 0.45
            to: 1.0
            duration: Math.max(1, skeleton.theme.motionNormal)
            easing.type: Easing.InOutSine
        }

        NumberAnimation {
            target: skeleton
            property: "pulse"
            from: 1.0
            to: 0.45
            duration: Math.max(1, skeleton.theme.motionNormal)
            easing.type: Easing.InOutSine
        }
    }

    onVisibleChanged: {
        if (!visible)
            pulse = 1
    }
}
