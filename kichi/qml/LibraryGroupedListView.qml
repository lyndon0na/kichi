import QtQuick
import QtQuick.Controls

// 播放历史表格：分区头独立于文件索引，和网格共用精确行布局。
Item {
    id: root

    property var libraryPage: null
    property var density: null
    property var metrics: null
    property int currentIndex: -1
    readonly property var entries: libraryPage && libraryPage.active
                                     ? libraryPage.entries : []
    readonly property int count: entries.length
    readonly property var rows: {
        const result = []
        let previous = ""
        for (let i = 0; i < entries.length; ++i) {
            const entry = entries[i]
            const group = entry.dateGroup === "today" ? "today"
                          : entry.dateGroup === "yesterday" ? "yesterday" : "earlier"
            if (group !== previous) {
                result.push({ key: "header-" + group, rowType: "header",
                              group: group, entryIndex: -1 })
                previous = group
            }
            result.push({ key: "entry-" + entry.id, rowType: "item",
                          entry: entry, entryIndex: i })
        }
        return result
    }

    property alias contentY: list.contentY
    property alias contentHeight: list.contentHeight

    signal keyPressed(var event)

    function forceActiveFocus() {
        list.forceActiveFocus()
    }

    function forceLayout() {
        list.forceLayout()
    }

    function rowIndexForEntry(entryIndex) {
        for (let i = 0; i < rows.length; ++i) {
            if (rows[i].entryIndex === entryIndex && rows[i].rowType !== "header")
                return i
        }
        return -1
    }

    function itemAtIndex(entryIndex) {
        const rowItem = list.itemAtRow(rowIndexForEntry(entryIndex))
        return rowItem ? rowItem.entryItem : null
    }

    function indexAt(x, y) {
        if (x < 0 || x >= width - metrics.scrollBarSlot)
            return -1
        const rowIndex = list.rowIndexAt(y)
        return rowIndex >= 0 ? rows[rowIndex].entryIndex : -1
    }

    function positionViewAtIndex(entryIndex, mode) {
        list.positionViewAtRow(rowIndexForEntry(entryIndex), mode)
    }

    LibrarySectionView {
        id: list

        objectName: "libraryDateList"
        anchors.fill: parent
        rows: root.rows
        itemRowHeight: root.density ? root.density.rowHeight : 48
        headerHeight: root.density ? root.density.groupHeaderHeight : 38
        onKeyPressed: function (event) { root.keyPressed(event) }

        TapHandler {
            acceptedButtons: Qt.RightButton
            onTapped: function (point) {
                if (!root.libraryPage)
                    return
                const viewport = list.mapFromItem(null, point.scenePosition.x,
                                                   point.scenePosition.y)
                if (root.libraryPage.rowAt(viewport.x, viewport.y))
                    return
                root.libraryPage.openBlankMenu(list.mapToItem(
                                                  null, viewport.x, viewport.y))
            }
        }

        delegate: Item {
            id: visualRow

            required property var rowData
            required property real rowOffset
            required property real rowExtent
            property alias entryItem: row

            y: rowOffset
            width: list.width - (root.metrics ? root.metrics.scrollBarSlot : 10)
            height: rowExtent

            LibraryDateSection {
                visible: visualRow.rowData.rowType === "header"
                objectName: visible ? "libraryDateSection_" + visualRow.rowData.group : ""
                width: visualRow.width
                height: list.headerHeight
                title: visible && root.libraryPage
                       ? root.libraryPage.dateSectionTitle(visualRow.rowData.group) : ""
                contentPadding: root.density ? root.density.rowPadding : 12
                rightPadding: contentPadding
            }

            LibraryRow {
                id: row

                visible: visualRow.rowData.rowType !== "header"
                width: visualRow.width
                height: list.itemRowHeight
                entry: visible ? visualRow.rowData.entry : null
                index: visualRow.rowData.entryIndex
                rowIndex: index
                backend: root.libraryPage ? root.libraryPage.backend : null
                highlighted: index >= 0 && index === root.currentIndex
                thumbnailSlotWidth: root.density.thumbnailSlotWidth
                thumbnailSlotHeight: root.density.thumbnailSlotHeight
                primaryColumnWidth: root.density.sizeColumnWidth
                secondaryColumnWidth: root.density.modifiedColumnWidth
                columnSpacing: root.density.columnSpacing
                contentPadding: root.density.rowPadding
                rowHeight: root.density.rowHeight
                textPixelSize: root.density.namePixelSize
                metaPixelSize: root.density.metaPixelSize
                iconPixelSize: root.density.iconPixelSize
                videoBadgeSize: root.density.listVideoBadgeSize
                videoBadgeMargin: root.density.listVideoBadgeMargin
                playLoading: entry && root.libraryPage
                             ? root.libraryPage.isPlayLoading(entry.id) : false
                playing: entry && root.libraryPage
                         ? root.libraryPage.isPlaying(entry.id) : false

                onActivateRequested: function (id) {
                    if (backend)
                        backend.openLibraryEntry(id)
                }
                onSelectRequested: function (index) {
                    if (root.libraryPage)
                        root.libraryPage.selectIndex(index)
                }
                onPlayRequested: function (id, point) {
                    if (root.libraryPage)
                        root.libraryPage.playEntry(id, row.mapToItem(null, point.x, point.y))
                }
                onContextMenu: function (point) {
                    if (root.libraryPage)
                        root.libraryPage.openEntryMenu(entry, row.mapToItem(null, point.x, point.y))
                }
            }
        }
    }

    ScrollFirstClickArea {
        id: firstClickArea
        view: list
        scrollBarSlot: root.metrics ? root.metrics.scrollBarSlot : 10
        hitTest: function (x, y) {
            const rowIndex = list.rowIndexAt(y + list.contentY)
            if (rowIndex < 0 || rowIndex >= root.rows.length)
                return -1
            const rowData = root.rows[rowIndex]
            return rowData && rowData.rowType !== "header"
                   ? rowData.entryIndex : -1
        }

        onClickedAt: function (index, modifiers, position) {
            const item = root.itemAtIndex(index)
            if (!item)
                return
            list.forceActiveFocus()
            firstClickArea.invokeFirstClick(item, index, modifiers,
                                            position.x, position.y)
        }
    }
}
