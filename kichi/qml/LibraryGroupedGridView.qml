import QtQuick
import QtQuick.Controls

// 播放历史专用网格：每次只实例化视口附近的卡片行，时间分区带可横跨整行。
// 普通文件页和另外两个资料库页继续使用 ContentGridView。
Item {
    id: root

    property var libraryPage: null
    property var entries: libraryPage && libraryPage.active
                            ? libraryPage.entries : []
    property var density: null
    property var metrics: null
    property int currentIndex: -1
    property var rows: []
    property bool rowsDirty: true

    signal keyPressed(var event)

    readonly property int sideMargin: metrics ? metrics.pageMargin : 12
    readonly property int scrollBarSlot: metrics ? metrics.scrollBarSlot : 10
    readonly property real minItemWidth: density ? density.gridMinItemWidth : 160
    readonly property int columnGap: density ? density.gridColumnGap : 14
    readonly property int rowGap: density ? density.gridRowGap : 18
    readonly property int headerHeight: density ? density.groupHeaderHeight : 38
    // 最后一列 cell 尾部自带 columnGap，右 margin 不再重复预留。
    // 滚动条悬浮在页面右侧留白里，不参与布局，右缘始终收在页面边距上。
    readonly property int rightMargin: Math.max(
        0, sideMargin - columnGap)
    // 列数按再让出一条车道的宽度估算，给悬浮条留出余量，也避免窗口宽度临界时抖列。
    readonly property int columnBasisRightMargin: Math.max(
        0, sideMargin + scrollBarSlot - columnGap)
    readonly property real layoutAvailableWidth: Math.max(
        0, width - sideMargin - columnBasisRightMargin)
    readonly property int columns: Math.max(1, Math.floor(
        (layoutAvailableWidth + columnGap) / (minItemWidth + columnGap)))
    readonly property real availableWidth: Math.max(
        0, width - sideMargin - rightMargin)
    readonly property real cellSlotWidth: Math.max(
        1, Math.floor(Math.max(0, availableWidth - 1) / columns))
    readonly property real remainder: Math.max(
        0, availableWidth - columns * cellSlotWidth)
    // 卡片吸收末列尾间距和取整余量，使左右边距严格一致。
    readonly property real itemWidth: Math.max(
        1, cellSlotWidth - sideMargin + remainder)
    readonly property real itemHeight: density
        ? density.gridItemHeight(itemWidth, true) : 1
    readonly property real cellHeight: itemHeight + rowGap
    readonly property int count: entries ? entries.length : 0

    property alias contentY: groupedList.contentY
    property alias contentHeight: groupedList.contentHeight

    function forceActiveFocus() {
        groupedList.forceActiveFocus()
    }

    function forceLayout() {
        groupedList.forceLayout()
    }

    function rebuildRows() {
        if (!visible) {
            rowsDirty = true
            return
        }
        const groups = [
            { key: "today", cards: [] },
            { key: "yesterday", cards: [] },
            { key: "earlier", cards: [] }
        ]
        const source = entries || []
        for (let i = 0; i < source.length; ++i) {
            const entry = source[i]
            const key = entry && String(entry.dateGroup || "")
            const groupIndex = key === "today" ? 0
                               : key === "yesterday" ? 1 : 2
            groups[groupIndex].cards.push({
                entry: entry,
                entryIndex: i
            })
        }

        const next = []
        for (let groupIndex = 0; groupIndex < groups.length; ++groupIndex) {
            const group = groups[groupIndex]
            if (group.cards.length === 0)
                continue
            next.push({
                rowType: "header",
                key: group.key,
                title: libraryPage
                       ? libraryPage.dateSectionTitle(group.key) : group.key
            })
            for (let start = 0; start < group.cards.length; start += columns) {
                next.push({
                    rowType: "items",
                    key: group.key + "-" + start,
                    cards: group.cards.slice(start, start + columns)
                })
            }
        }
        rows = next
        rowsDirty = false
        groupedList.forceLayout()
    }

    function rowIndexForEntry(entryIndex) {
        for (let rowIndex = 0; rowIndex < rows.length; ++rowIndex) {
            const modelRow = rows[rowIndex]
            if (modelRow.rowType !== "items")
                continue
            for (let cardIndex = 0; cardIndex < modelRow.cards.length; ++cardIndex) {
                if (modelRow.cards[cardIndex].entryIndex === entryIndex)
                    return rowIndex
            }
        }
        return -1
    }

    function itemAtIndex(entryIndex) {
        const rowIndex = rowIndexForEntry(entryIndex)
        if (rowIndex < 0)
            return null
        const rowItem = groupedList.itemAtRow(rowIndex)
        if (!rowItem)
            return null
        const targetName = "libraryGridCard_" + entryIndex
        for (let childIndex = 0; childIndex < rowItem.children.length; ++childIndex) {
            const child = rowItem.children[childIndex]
            if (child && String(child.objectName) === targetName)
                return child
        }
        return null
    }

    /// 视口坐标 (x, y) 下命中的卡片；分区头和行列缝隙返回 null（按空白处理）。
    function cardAt(x, y) {
        const rowIndex = groupedList.rowIndexAt(y + groupedList.contentY)
        if (rowIndex < 0 || rowIndex >= rows.length)
            return null
        const modelRow = rows[rowIndex]
        if (!modelRow || modelRow.rowType !== "items")
            return null
        for (let i = 0; i < modelRow.cards.length; ++i) {
            const card = itemAtIndex(modelRow.cards[i].entryIndex)
            if (!card)
                continue
            const local = groupedList.mapToItem(card, x, y)
            if (local.x >= 0 && local.y >= 0
                    && local.x < card.width && local.y < card.height)
                return card
        }
        return null
    }

    function positionViewAtIndex(entryIndex, mode) {
        const rowIndex = rowIndexForEntry(entryIndex)
        if (rowIndex >= 0)
            groupedList.positionViewAtRow(rowIndex, mode)
    }

    onEntriesChanged: {
        if (!libraryPage || !libraryPage.active) {
            rows = []
            rowsDirty = true
        } else if (visible) {
            rebuildRows()
        } else {
            rowsDirty = true
        }
    }
    onColumnsChanged: {
        if (visible)
            rebuildRows()
        else
            rowsDirty = true
    }
    onVisibleChanged: {
        if (visible && rowsDirty)
            rebuildRows()
    }
    Component.onCompleted: {
        if (visible)
            rebuildRows()
    }

    LibrarySectionView {
        id: groupedList

        objectName: "libraryGroupedList"
        anchors.fill: parent
        rows: root.rows
        itemRowHeight: root.cellHeight
        headerHeight: root.headerHeight

        Keys.onPressed: function (event) {
            root.keyPressed(event)
        }

        TapHandler {
            acceptedButtons: Qt.RightButton
            onTapped: function (point) {
                if (!root.libraryPage)
                    return
                // TapHandler 挂在 Flickable 上时 position 以内容区为原点，纵向随
                // contentY 走。先换算成视口坐标，
                // 否则滚动越深弹窗越偏，最后被夹到窗口底部。
                const viewport = groupedList.mapFromItem(
                    null, point.scenePosition.x, point.scenePosition.y)
                if (root.cardAt(viewport.x, viewport.y) !== null)
                    return
                root.libraryPage.openBlankMenu(
                    groupedList.mapToItem(null, viewport.x, viewport.y))
            }
        }

        delegate: Item {
            id: visualRow

            required property var rowData
            required property real rowOffset
            required property real rowExtent

            y: rowOffset
            width: groupedList.width
            height: rowExtent

            LibraryDateSection {
                objectName: "libraryDateSection_" + visualRow.rowData.key
                visible: visualRow.rowData.rowType === "header"
                width: visualRow.width
                height: root.headerHeight
                title: visualRow.rowData.rowType === "header"
                       ? visualRow.rowData.title : ""
                contentPadding: root.sideMargin
                rightPadding: root.sideMargin
            }

            Repeater {
                id: cardRepeater

                model: visualRow.rowData.rowType === "items"
                       ? visualRow.rowData.cards.length : 0

                delegate: LibraryGridCard {
                    readonly property var cardData:
                        visualRow.rowData.cards[index]

                    objectName: "libraryGridCard_" + cardData.entryIndex
                    x: root.sideMargin + index * root.cellSlotWidth
                    width: root.itemWidth
                    height: root.itemHeight
                    libraryPage: root.libraryPage
                    entryData: cardData.entry
                    entryIndex: cardData.entryIndex
                    density: root.density

                    onEntrySelected: function (entryIndex, modifiers) {
                        root.currentIndex = entryIndex
                        if (root.libraryPage)
                            root.libraryPage.selectIndex(entryIndex)
                    }
                    onEntryFolderOpenRequested: function (id) {
                        if (root.libraryPage && root.libraryPage.backend)
                            root.libraryPage.backend.openLibraryEntry(id)
                    }
                    onEntryPlayRequested: function (id, position) {
                        if (root.libraryPage)
                            root.libraryPage.playEntry(id, position)
                    }
                    onEntryContextMenuRequested: function (entry, position) {
                        if (!root.libraryPage)
                            return
                        root.libraryPage.selectIndex(entryIndex, false)
                        groupedList.forceActiveFocus()
                        root.libraryPage.openEntryMenu(entry, position)
                    }
                }
            }
        }
    }

    ScrollFirstClickArea {
        id: firstClickArea
        view: groupedList
        scrollBarSlot: root.scrollBarSlot
        hitTest: function (x, y) {
            const card = root.cardAt(x, y)
            return card ? card.entryIndex : -1
        }

        onClickedAt: function (index, modifiers, position) {
            const item = root.itemAtIndex(index)
            if (!item)
                return
            groupedList.forceActiveFocus()
            firstClickArea.invokeFirstClick(item, index, modifiers,
                                            position.x, position.y)
        }
    }
}
