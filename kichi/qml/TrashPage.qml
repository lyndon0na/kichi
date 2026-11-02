import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 回收站表格：查看 / 批量还原 / 彻底删除 / 清空回收站。
// 普通文件页不暴露彻底删除或清空，危险操作只能从这里进入。
Item {
    id: page
    property var backend: null

    FileSelection {
        id: selection
        objectName: "trashSelection"
    }
    FileListDensity {
        id: density
        objectName: "fileListDensity"
    }
    UiMetrics { id: metrics }
    KTheme { id: theme }

    readonly property var entries: backend ? backend.trashFiles : []
    readonly property var trashRows: entryRows.model
    readonly property int selectedCount: selection.count
    readonly property bool busy: backend !== null && (backend.trashLoading || backend.fileActionRunning)
    readonly property bool backendSessionActive: page.backend === null
                                                 || typeof page.backend.authState === "undefined"
                                                 || page.backend.authState === "signedIn"
    readonly property bool showInitialSkeleton: initialLoadGate.showSkeleton
    readonly property bool gridMode: backend !== null
                                     && backend.contentView === "grid"
    readonly property var activeView: gridMode ? trashGrid : trashList
    property var scrollByView: ({ "list": 0, "grid": 0 })
    property bool pendingScrollRestore: false
    property real pendingScrollY: 0

    EntryRows {
        id: entryRows
        source: page.entries
    }

    FirstLoadGate {
        id: initialLoadGate

        active: page.visible
        sessionActive: page.backendSessionActive
        loading: page.backend !== null && page.backend.trashLoading
        count: page.entries.length
    }

    function selectedEntry() {
        const ids = selection.idList()
        if (ids.length === 0)
            return null
        for (let i = 0; i < entries.length; i++) {
            if (entries[i].id === ids[0])
                return entries[i]
        }
        return null
    }

    function restoreSelected() {
        if (!backend || busy || selection.count === 0)
            return
        backend.untrashEntries(selection.idList())
    }

    function deleteSelected() {
        if (!backend || busy || selection.count === 0)
            return
        const entry = selectedEntry()
        if (!entry)
            return
        dialogs.openDelete(entry, selection.idList())
    }

    function clearAll() {
        if (!backend || busy || entries.length === 0)
            return
        dialogs.openClear(entries.length)
    }

    function openContext(point) {
        contextMenu.popup(point)
    }

    function rememberScroll() {
        if (!page.activeView)
            return
        const next = ({})
        const keys = Object.keys(page.scrollByView)
        for (let i = 0; i < keys.length; ++i)
            next[keys[i]] = page.scrollByView[keys[i]]
        next[page.gridMode ? "grid" : "list"] = page.activeView.contentY
        page.scrollByView = next
    }

    function setContentView(view) {
        if (!page.backend || typeof page.backend.setContentView !== "function")
            return
        const next = view === "grid" ? "grid" : "list"
        if (page.backend.contentView === next)
            return
        page.rememberScroll()
        page.backend.setContentView(next)
    }

    function restoreScroll() {
        if (!page.pendingScrollRestore || !page.activeView)
            return
        if (page.activeView.contentHeight <= 0)
            return
        const maxY = Math.max(0, page.activeView.contentHeight
                                 - page.activeView.height)
        page.activeView.contentY = Math.max(0, Math.min(page.pendingScrollY, maxY))
        page.pendingScrollRestore = false
    }

    function focusActiveView() {
        if (page.visible && page.activeView)
            page.activeView.forceActiveFocus()
    }

    function handleKey(event) {
        if (!backend)
            return
        if (event.key === Qt.Key_A && (event.modifiers & Qt.ControlModifier)) {
            selection.selectAll(entries)
            event.accepted = true
            return
        }
        if (event.key === Qt.Key_Delete) {
            deleteSelected()
            event.accepted = true
            return
        }
        if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
            restoreSelected()
            event.accepted = true
        }
    }

    onGridModeChanged: {
        page.pendingScrollY = page.scrollByView[page.gridMode ? "grid" : "list"] || 0
        page.pendingScrollRestore = true
        Qt.callLater(page.restoreScroll)
        Qt.callLater(page.focusActiveView)
    }

    onVisibleChanged: {
        if (!visible) {
            page.rememberScroll()
            return
        }
        if (backend) {
            if (typeof backend.ensureTrashFresh === "function")
                backend.ensureTrashFresh()
            else
                backend.refreshTrash()
            Qt.callLater(initialLoadGate.start)
        }
        if (!page.pendingScrollRestore) {
            page.pendingScrollY = page.scrollByView[page.gridMode ? "grid" : "list"] || 0
            page.pendingScrollRestore = true
        }
        Qt.callLater(function () {
            page.restoreScroll()
            page.focusActiveView()
        })
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.topMargin: metrics.pageMargin
        spacing: 8

        Label {
            objectName: "trashError"
            Layout.fillWidth: true
            Layout.leftMargin: metrics.pageMargin
            Layout.rightMargin: metrics.pageMargin + metrics.scrollBarSlot
            visible: text.length > 0
            text: page.backend ? page.backend.trashError : ""
            color: theme.danger
            font.pixelSize: theme.fontMeta
            wrapMode: Text.Wrap
        }

        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true

            ColumnLayout {
                anchors.fill: parent
                spacing: 4
                visible: !page.gridMode

                TrashTableHeader {
                    Layout.fillWidth: true
                    thumbnailSlotWidth: density.thumbnailSlotWidth
                    deletedColumnWidth: density.modifiedColumnWidth
                    sizeColumnWidth: density.sizeColumnWidth
                    contentPadding: density.rowPadding
                    columnSpacing: density.columnSpacing
                    headerRowHeight: density.headerRowHeight
                    headerPixelSize: density.metaPixelSize
                }

                Item {
                    Layout.fillWidth: true
                    Layout.fillHeight: true

                    ListView {
                        id: trashList
                        objectName: "trashList"

                        anchors.fill: parent
                        visible: !page.gridMode
                        clip: true
                        spacing: 0
                        model: page.trashRows
                        boundsBehavior: Flickable.StopAtBounds
                        ScrollBar.vertical: KScrollBar {}
                        focus: true
                        onVisibleChanged: {
                            if (visible)
                                forceActiveFocus()
                        }
                        Keys.onPressed: function (event) {
                            page.handleKey(event)
                        }
                        onContentHeightChanged: page.restoreScroll()

                        delegate: TrashRow {
                            id: row
                            width: ListView.view.width - metrics.scrollBarSlot
                            backend: page.backend
                            rowIndex: index
                            highlighted: !!entry && selection.isSelected(entry.id)
                            thumbnailSlotWidth: density.thumbnailSlotWidth
                            thumbnailSlotHeight: density.thumbnailSlotHeight
                            deletedColumnWidth: density.modifiedColumnWidth
                            sizeColumnWidth: density.sizeColumnWidth
                            columnSpacing: density.columnSpacing
                            contentPadding: density.rowPadding
                            rowHeight: density.rowHeight
                            textPixelSize: density.namePixelSize
                            metaPixelSize: density.metaPixelSize
                            iconPixelSize: density.iconPixelSize

                            onSelectRequested: function (modifiers, index) {
                                if (index < 0 || index >= page.entries.length)
                                    return
                                selection.click(page.entries[index].id, index,
                                                modifiers, page.entries)
                            }

                            onContextMenu: function (point) {
                                if (!entry)
                                    return
                                selection.ensureSelected(entry)
                                trashList.forceActiveFocus()
                                var windowPoint = row.mapToItem(null, point.x, point.y)
                                page.openContext(windowPoint)
                            }
                        }
                    }

                    ScrollFirstClickArea {
                        id: trashListFirstClick
                        view: trashList
                        scrollBarSlot: metrics.scrollBarSlot

                        onClickedAt: function (index, modifiers, position) {
                            const item = trashList.itemAtIndex(index)
                            if (!item)
                                return
                            trashList.forceActiveFocus()
                            trashListFirstClick.invokeFirstClick(
                                item, index, modifiers, position.x, position.y)
                        }
                    }
                }
            }

            ContentGridView {
                id: trashGrid

                anchors.fill: parent
                visible: page.gridMode
                model: page.trashRows
                density: density
                metrics: metrics
                reserveDetail: true
                viewObjectName: "trashGrid"

                onKeyPressed: function (event) {
                    page.handleKey(event)
                }
                onContentHeightChanged: page.restoreScroll()

                delegate: Component {
                    FileGridItem {
                        id: trashGridItem

                        width: trashGrid.itemWidth
                        height: trashGrid.itemHeight
                        backend: page.backend
                        rowIndex: index
                        activationEnabled: false
                        selected: !!entry && selection.isSelected(entry.id)
                        contentPadding: density.gridCardPadding
                        cardSpacing: density.gridCardSpacing
                        nameLineHeight: density.gridNameLineHeight
                        nameBlockHeight: density.gridNameBlockHeight
                        thumbnailRadius: density.gridThumbnailRadius(width)
                        thumbnailDecodeWidth: density.gridThumbnailDecodeWidth
                        thumbnailDecodeHeight: density.gridThumbnailDecodeHeight
                        textPixelSize: density.namePixelSize
                        detailPixelSize: density.metaPixelSize
                        detailLineHeight: density.gridDetailLineHeight
                        iconPixelSize: density.gridIconPixelSize
                        videoBadgeSize: density.gridVideoBadgeSize
                        videoBadgeMargin: density.gridVideoBadgeMargin
                        detailText: entry
                                    ? qsTr("删除于 %1 · %2").arg(entry.deletedText)
                                      .arg(entry.sizeText) : qsTr("删除时间未知")

                        onSelectRequested: function (modifiers, itemIndex) {
                            if (itemIndex < 0 || itemIndex >= page.entries.length)
                                return
                            trashGrid.currentIndex = itemIndex
                            selection.click(page.entries[itemIndex].id, itemIndex,
                                            modifiers, page.entries)
                        }
                        onContextMenu: function (point) {
                            if (!entry)
                                return
                            selection.ensureSelected(entry)
                            trashGrid.forceActiveFocus()
                            page.openContext(trashGridItem.mapToItem(
                                                 null, point.x, point.y))
                        }
                    }
                }
            }

            LoadingSkeleton {
                anchors.fill: parent
                visible: page.showInitialSkeleton
                gridMode: page.gridMode
                density: density
                metrics: metrics
                gridView: page.gridMode ? trashGrid : null
                listTopInset: page.gridMode ? 0 : density.headerRowHeight + 4
                trailingColumnWidths: [density.modifiedColumnWidth,
                                       density.sizeColumnWidth]
            }

            BusyIndicator {
                objectName: "trashLoadingIndicator"
                anchors.centerIn: parent
                visible: page.backend && page.backend.trashLoading
                         && !page.showInitialSkeleton
                running: visible
                width: 28
                height: 28
            }

            Label {
                objectName: "trashEmptyHint"
                anchors.centerIn: parent
                visible: page.backend && page.backend.trashLoaded
                         && !page.backend.trashLoading
                         && page.entries.length === 0
                         && page.backend.trashError.length === 0
                text: qsTr("回收站是空的")
                color: theme.textMuted
                font.pixelSize: theme.fontBody
            }

            Label {
                anchors.centerIn: parent
                visible: page.backend && !page.backend.trashLoaded
                         && !page.backend.trashLoading
                         && page.backend.trashError.length > 0
                text: qsTr("回收站读取失败\n请点刷新重试")
                horizontalAlignment: Text.AlignHCenter
                color: theme.textMuted
                font.pixelSize: theme.fontBody
            }
        }
    }

    Menu {
        id: contextMenu
        objectName: "trashContextMenu"
        parent: Overlay.overlay
        width: 190

        MenuItem {
            text: qsTr("还原")
            enabled: page.selectedCount > 0 && !page.busy
            onTriggered: page.restoreSelected()
        }

        MenuItem {
            objectName: "trashDeleteMenuItem"
            text: qsTr("彻底删除")
            enabled: page.selectedCount > 0 && !page.busy
            onTriggered: page.deleteSelected()
        }
    }

    TrashActionDialogs {
        id: dialogs
        backend: page.backend
    }

    Connections {
        target: page.backend

        function onChanged() {
            if (!page.backend)
                return
            // 乐观删除 / 回滚期间保留选择；成功后再由 fileActionDone 清空。
            if (!page.backend.fileActionRunning)
                selection.prune(page.entries)
        }

        function onFileActionDone(action, ok) {
            if (!ok)
                return
            if (action === "trash") {
                if (page.visible && typeof page.backend.ensureTrashFresh === "function")
                    page.backend.ensureTrashFresh()
                return
            }
            if (action === "untrash" || action === "trashDelete" || action === "trashClear")
                selection.clear()
        }
    }
}
