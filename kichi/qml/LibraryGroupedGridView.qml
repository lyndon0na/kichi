import QtQuick
import QtQuick.Controls

// 播放历史专用网格：ListView 每次只实例化一小排卡片，时间分区带可横跨整行。
// 普通文件页和另外两个资料库页继续使用 ContentGridView。
Item {
    id: root

    property var libraryPage: null
    property var entries: libraryPage ? libraryPage.entries : []
    property var density: null
    property var metrics: null
    property int currentIndex: -1
    property var rows: []
    property alias rowModel: rowList

    signal keyPressed(var event)

    readonly property int sideMargin: metrics ? metrics.pageMargin : 12
    readonly property int scrollBarSlot: metrics ? metrics.scrollBarSlot : 10
    readonly property real availableWidth: Math.max(
        0, width - sideMargin * 2 - scrollBarSlot)
    readonly property real minItemWidth: density ? density.gridMinItemWidth : 160
    readonly property int columnGap: density ? density.gridColumnGap : 14
    readonly property int rowGap: density ? density.gridRowGap : 18
    readonly property int columns: Math.max(1, Math.floor(
        (availableWidth + columnGap) / (minItemWidth + columnGap)))
    readonly property real cellSlotWidth: columns > 0
        ? Math.max(1, Math.floor(availableWidth / columns)) : 0
    readonly property real itemWidth: Math.max(1, cellSlotWidth - columnGap)
    readonly property real itemHeight: density
        ? density.gridItemHeight(itemWidth, true) : 1
    readonly property real cellHeight: itemHeight + rowGap
    readonly property int headerHeight: density ? density.groupHeaderHeight : 38
    readonly property int count: entries ? entries.length : 0

    property alias contentY: groupedList.contentY
    property alias contentHeight: groupedList.contentHeight

    ListModel {
        id: rowList
    }

    function forceActiveFocus() {
        groupedList.forceActiveFocus()
    }

    function forceLayout() {
        groupedList.forceLayout()
    }

    function rebuildRows() {
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
        syncRowList(next)
        Qt.callLater(function () {
            groupedList.forceLayout()
        })
    }

    function syncRowList(next) {
        const wantedKeys = ({})
        for (let i = 0; i < next.length; ++i)
            wantedKeys[String(next[i].key)] = true

        for (let stale = rowList.count - 1; stale >= 0; --stale) {
            if (wantedKeys[String(rowList.get(stale).key)] === undefined)
                rowList.remove(stale)
        }

        const present = ({})
        for (let p = 0; p < rowList.count; ++p)
            present[String(rowList.get(p).key)] = true
        for (let add = 0; add < next.length; ++add) {
            const key = String(next[add].key)
            if (present[key] === undefined) {
                rowList.append({ key: key, rowData: next[add] })
                present[key] = true
            }
        }

        for (let target = next.length - 1; target >= 0 && target < rowList.count; --target) {
            const key = String(next[target].key)
            if (String(rowList.get(target).key) === key)
                continue
            let from = -1
            for (let scan = target - 1; scan >= 0; --scan) {
                if (String(rowList.get(scan).key) === key) {
                    from = scan
                    break
                }
            }
            if (from >= 0)
                rowList.move(from, target, 1)
        }

        for (let at = 0; at < next.length && at < rowList.count; ++at) {
            rowList.set(at, { key: String(next[at].key), rowData: next[at] })
        }
        while (rowList.count > next.length)
            rowList.remove(rowList.count - 1)
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
        const rowItem = groupedList.itemAtIndex(rowIndex)
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

    function positionViewAtIndex(entryIndex, mode) {
        const rowIndex = rowIndexForEntry(entryIndex)
        if (rowIndex >= 0)
            groupedList.positionViewAtIndex(rowIndex, mode)
    }

    onEntriesChanged: rebuildRows()
    onColumnsChanged: rebuildRows()
    onVisibleChanged: {
        if (visible)
            Qt.callLater(function () {
                groupedList.forceLayout()
            })
    }
    Component.onCompleted: rebuildRows()

    ListView {
        id: groupedList

        objectName: "libraryGroupedList"
        anchors.fill: parent
        clip: true
        spacing: 0
        model: root.rowModel
        currentIndex: -1
        boundsBehavior: Flickable.StopAtBounds
        ScrollBar.vertical: KScrollBar {}
        focus: true

        Keys.onPressed: function (event) {
            root.keyPressed(event)
        }

        TapHandler {
            acceptedButtons: Qt.RightButton
            onTapped: function (point) {
                if (!root.libraryPage)
                    return
                const x = point.position.x
                const y = point.position.y
                if (groupedList.indexAt(x, y + groupedList.contentY) >= 0)
                    return
                root.libraryPage.openBlankMenu(
                    groupedList.mapToItem(null, x, y))
            }
        }

        delegate: Item {
            id: visualRow

            required property var rowData

            width: groupedList.width - root.scrollBarSlot
            height: rowData.rowType === "header"
                    ? root.headerHeight : root.cellHeight

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
}
