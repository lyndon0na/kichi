import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 最近保存 / 播放历史 / 已加星标的公共列表 / 网格骨架。
// 数据来源、空态文案与两列含义由三个薄页面分别提供。
Item {
    id: page

    property var backend: null
    property bool active: false
    property string kind: ""
    property string primaryHeader: ""
    property string secondaryHeader: ""
    property string emptyText: ""
    property bool dateSectionsEnabled: false
    property alias contextMenu: libraryMenu.menu
    property alias blankContextMenu: libraryMenu.blankMenu

    readonly property bool activeCollection: page.backend !== null
                                            && page.backend.libraryKind === page.kind
    readonly property var entries: page.activeCollection ? page.backend.libraryRows : []
    readonly property bool loading: page.activeCollection && page.backend.libraryLoading
    readonly property bool loaded: page.activeCollection && page.backend.libraryLoaded
    readonly property string errorText: page.activeCollection ? page.backend.libraryError : ""
    property int lastRevision: -1
    property string currentEntryId: ""
    readonly property bool gridMode: page.backend !== null
                                     && page.backend.contentView === "grid"
    readonly property bool dateGrouped: page.dateSectionsEnabled
                                        && page.kind === "playback-history"
    readonly property var activeView: page.gridMode
                                      ? (page.dateGrouped ? libraryGroupedGrid : libraryGrid)
                                      : libraryList
    property var scrollByView: ({ "list": 0, "grid": 0 })
    property bool pendingScrollRestore: false
    property real pendingScrollY: 0

    FileListDensity {
        id: density
        objectName: "libraryDensity"
    }
    UiMetrics { id: metrics }
    KTheme { id: theme }

    function isPlayLoading(id) {
        return page.backend !== null
                && page.backend.playLoading === true
                && String(page.backend.playLoadingId) === String(id)
    }

    function isPlaying(id) {
        const ids = page.backend && page.backend.playingIds
                    ? page.backend.playingIds : []
        for (let i = 0; i < ids.length; ++i) {
            if (String(ids[i]) === String(id))
                return true
        }
        return false
    }

    function refresh() {
        if (!page.backend || page.kind.length === 0 || page.loading)
            return
        page.backend.refreshLibrary(page.kind)
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
        if (typeof page.activeView.forceLayout === "function")
            page.activeView.forceLayout()
        if (page.activeView.contentHeight <= 0)
            return
        const maxY = Math.max(0, page.activeView.contentHeight
                                 - page.activeView.height)
        page.activeView.contentY = Math.max(0, Math.min(page.pendingScrollY, maxY))
        page.pendingScrollRestore = false
    }

    function focusActiveView() {
        if (page.active && page.activeView)
            page.activeView.forceActiveFocus()
    }

    function gridDetailText(entry) {
        if (!entry)
            return qsTr("—")
        const parts = []
        if (entry.primaryText)
            parts.push(String(entry.primaryText))
        if (entry.secondaryText)
            parts.push(String(entry.secondaryText))
        return parts.length > 0 ? parts.join(" · ") : qsTr("—")
    }

    function dateSectionTitle(key) {
        if (key === "today")
            return qsTr("今天")
        if (key === "yesterday")
            return qsTr("昨天")
        return qsTr("更早")
    }

    function playEntry(id, anchorPoint) {
        if (!page.backend)
            return
        playMenu.anchorPoint = anchorPoint
        page.backend.requestPlay(id)
    }

    function activateCurrent() {
        if (!page.backend || entries.length === 0)
            return
        const index = Math.max(0, Math.min(
            page.activeView.currentIndex,
            entries.length - 1))
        const entry = entries[index]
        if (!entry)
            return
        if (entry.kind === "folder") {
            page.backend.openLibraryEntry(entry.id)
        } else if (entry.playable === true) {
            page.playEntry(entry.id, page.activeView.mapToItem(
                               null,
                               page.activeView.width / 2,
                               Math.max(24, page.activeView.height / 2)))
        }
    }

    function selectIndex(index, focus) {
        if (entries.length === 0) {
            libraryList.currentIndex = -1
            page.currentEntryId = ""
            libraryGrid.currentIndex = -1
            libraryGroupedGrid.currentIndex = -1
            return
        }
        const next = Math.max(0, Math.min(index, entries.length - 1))
        libraryList.currentIndex = next
        libraryGrid.currentIndex = next
        libraryGroupedGrid.currentIndex = next
        const entry = entries[next]
        page.currentEntryId = entry && entry.id ? String(entry.id) : ""
        if (focus !== false)
            page.focusActiveView()
    }

    function indexOfCurrentEntry() {
        if (page.currentEntryId.length === 0)
            return -1
        for (let i = 0; i < page.entries.length; ++i) {
            const entry = page.entries[i]
            if (entry && String(entry.id) === page.currentEntryId)
                return i
        }
        return -1
    }

    function openEntryMenu(entry, point) {
        if (!page.backend || !entry)
            return
        libraryMenu.openFor(entry, point)
    }

    function openBlankMenu(point) {
        if (!page.backend)
            return
        libraryMenu.openBlankAt(point)
    }

    function moveSelection(index) {
        page.selectIndex(index)
        if (page.activeView && page.activeView.currentIndex >= 0)
            page.activeView.positionViewAtIndex(
                page.activeView.currentIndex, ListView.Contain)
    }

    function handleKey(event) {
        if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
            page.activateCurrent()
            event.accepted = true
        } else if (event.key === Qt.Key_Up) {
            page.moveSelection(page.activeView.currentIndex - 1)
            event.accepted = true
        } else if (event.key === Qt.Key_Down) {
            page.moveSelection(page.activeView.currentIndex + 1)
            event.accepted = true
        } else if (event.key === Qt.Key_Left) {
            page.moveSelection(page.activeView.currentIndex - 1)
            event.accepted = true
        } else if (event.key === Qt.Key_Right) {
            page.moveSelection(page.activeView.currentIndex + 1)
            event.accepted = true
        } else if (event.key === Qt.Key_Home) {
            page.moveSelection(0)
            event.accepted = true
        } else if (event.key === Qt.Key_End) {
            page.moveSelection(page.entries.length - 1)
            event.accepted = true
        }
    }

    onActiveChanged: {
        if (!active) {
            page.rememberScroll()
            playMenu.close()
            page.contextMenu.close()
            page.blankContextMenu.close()
            return
        }
        page.refresh()
        if (!page.pendingScrollRestore) {
            page.pendingScrollY = page.scrollByView[page.gridMode ? "grid" : "list"] || 0
            page.pendingScrollRestore = true
        }
        Qt.callLater(function () {
            page.restoreScroll()
            page.focusActiveView()
        })
    }

    onGridModeChanged: {
        page.pendingScrollY = page.scrollByView[page.gridMode ? "grid" : "list"] || 0
        page.pendingScrollRestore = true
        const index = page.indexOfCurrentEntry()
        if (index >= 0) {
            libraryList.currentIndex = index
            libraryGrid.currentIndex = index
            libraryGroupedGrid.currentIndex = index
        }
        Qt.callLater(page.restoreScroll)
        Qt.callLater(page.focusActiveView)
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.topMargin: metrics.pageMargin
        spacing: 8

        Label {
            objectName: "libraryError"
            Layout.fillWidth: true
            Layout.leftMargin: metrics.pageMargin
            Layout.rightMargin: metrics.pageMargin + metrics.scrollBarSlot
            visible: text.length > 0
            text: page.errorText
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

                LibraryTableHeader {
                    Layout.fillWidth: true
                    thumbnailSlotWidth: density.thumbnailSlotWidth
                    primaryColumnWidth: density.sizeColumnWidth
                    secondaryColumnWidth: density.modifiedColumnWidth
                    contentPadding: density.rowPadding
                    columnSpacing: density.columnSpacing
                    headerRowHeight: density.headerRowHeight
                    headerPixelSize: density.metaPixelSize
                    primaryLabel: page.primaryHeader
                    secondaryLabel: page.secondaryHeader
                }

                ListView {
                    id: libraryList
                    objectName: "libraryList"

                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    visible: !page.gridMode
                    clip: true
                    spacing: 0
                    model: page.entries
                    currentIndex: -1
                    boundsBehavior: Flickable.StopAtBounds
                    ScrollBar.vertical: KScrollBar {}
                    focus: true
                    highlightFollowsCurrentItem: false
                    section.property: page.dateGrouped ? "dateGroup" : ""
                    section.criteria: ViewSection.FullString
                    section.delegate: LibraryDateSection {
                        objectName: "libraryDateSection_" + section
                        width: libraryList.width - metrics.scrollBarSlot
                        height: density.groupHeaderHeight
                        title: page.dateSectionTitle(section)
                        contentPadding: density.rowPadding
                        rightPadding: density.rowPadding
                    }
                    Keys.onPressed: function (event) {
                        page.handleKey(event)
                    }
                    onContentHeightChanged: page.restoreScroll()

                    TapHandler {
                        acceptedButtons: Qt.RightButton
                        onTapped: function (point) {
                            if (!page.backend)
                                return
                            const x = point.position.x
                            const y = point.position.y
                            if (libraryList.indexAt(x, y + libraryList.contentY) >= 0)
                                return
                            page.openBlankMenu(libraryList.mapToItem(null, x, y))
                        }
                    }

                    delegate: LibraryRow {
                        id: row
                        width: ListView.view.width - metrics.scrollBarSlot
                        entry: modelData
                        backend: page.backend
                        rowIndex: index
                        highlighted: ListView.isCurrentItem
                        thumbnailSlotWidth: density.thumbnailSlotWidth
                        thumbnailSlotHeight: density.thumbnailSlotHeight
                        primaryColumnWidth: density.sizeColumnWidth
                        secondaryColumnWidth: density.modifiedColumnWidth
                        columnSpacing: density.columnSpacing
                        contentPadding: density.rowPadding
                        rowHeight: density.rowHeight
                        textPixelSize: density.namePixelSize
                        metaPixelSize: density.metaPixelSize
                        iconPixelSize: density.iconPixelSize
                        videoBadgeSize: density.listVideoBadgeSize
                        videoBadgeMargin: density.listVideoBadgeMargin
                        playLoading: page.isPlayLoading(entry.id)
                        playing: page.isPlaying(entry.id)

                        onActivateRequested: function (id) {
                            if (page.backend)
                                page.backend.openLibraryEntry(id)
                        }
                        onSelectRequested: function (index) {
                            page.selectIndex(index)
                        }
                        onPlayRequested: function (id, point) {
                            page.playEntry(id, row.mapToItem(null, point.x, point.y))
                        }
                        onContextMenu: function (point) {
                            page.openEntryMenu(entry,
                                               row.mapToItem(null, point.x, point.y))
                        }
                    }
                }
            }

            ContentGridView {
                id: libraryGrid

                objectName: "libraryGridContainer"
                anchors.fill: parent
                visible: page.gridMode && !page.dateGrouped
                model: page.entries
                density: density
                metrics: metrics
                reserveDetail: true
                viewObjectName: page.dateGrouped ? "libraryUngroupedGrid" : "libraryGrid"

                onKeyPressed: function (event) {
                    page.handleKey(event)
                }
                onContentHeightChanged: page.restoreScroll()
                onBlankContextMenuRequested: function (point) {
                    if (!page.backend)
                        return
                    page.openBlankMenu(libraryGrid.mapToItem(
                                           null, point.x, point.y))
                }

                delegate: Component {
                    LibraryGridCard {
                        id: libraryGridItem

                        width: libraryGrid.itemWidth
                        height: libraryGrid.itemHeight
                        libraryPage: page
                        entryData: modelData
                        entryIndex: index
                        density: density

                        onEntrySelected: function (entryIndex, modifiers) {
                            if (entryIndex < 0 || entryIndex >= page.entries.length)
                                return
                            libraryGrid.currentIndex = entryIndex
                            page.selectIndex(entryIndex)
                        }
                        onEntryFolderOpenRequested: function (id) {
                            if (page.backend)
                                page.backend.openLibraryEntry(id)
                        }
                        onEntryPlayRequested: function (id, position) {
                            page.playEntry(id, position)
                        }
                        onEntryContextMenuRequested: function (entry, position) {
                            if (!entry)
                                return
                            page.selectIndex(libraryGridItem.rowIndex, false)
                            libraryGrid.forceActiveFocus()
                            page.openEntryMenu(entry, position)
                        }
                    }
                }
            }

            LibraryGroupedGridView {
                id: libraryGroupedGrid

                objectName: "libraryGroupedGrid"
                anchors.fill: parent
                visible: page.gridMode && page.dateGrouped
                libraryPage: page
                density: density
                metrics: metrics

                onKeyPressed: function (event) {
                    page.handleKey(event)
                }
                onContentHeightChanged: page.restoreScroll()
            }

            BusyIndicator {
                anchors.centerIn: parent
                visible: page.loading && page.entries.length === 0
                running: visible
                width: 28
                height: 28
            }

            Label {
                objectName: "libraryEmptyHint"
                anchors.centerIn: parent
                width: Math.min(parent.width - 40, 420)
                visible: page.loaded && !page.loading && page.entries.length === 0
                         && page.errorText.length === 0
                text: page.emptyText
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.Wrap
                color: theme.textMuted
                font.pixelSize: theme.fontBody
            }

            Label {
                objectName: "libraryFailureHint"
                anchors.centerIn: parent
                width: Math.min(parent.width - 40, 420)
                visible: page.activeCollection && !page.loaded && !page.loading
                         && page.errorText.length > 0 && page.entries.length === 0
                text: qsTr("资料库读取失败\n请稍后重试")
                horizontalAlignment: Text.AlignHCenter
                color: theme.textMuted
                font.pixelSize: theme.fontBody
            }
        }
    }

    FilesPagePlayMenu {
        id: playMenu
        objectName: "libraryPlayMenu"
        backend: page.active ? page.backend : null
    }

    LibraryContextMenu {
        id: libraryMenu
        backend: page.backend
        onPlayRequested: function (id, point) {
            page.playEntry(id, point)
        }
        onOpenFolderRequested: function (id) {
            if (page.backend)
                page.backend.openLibraryEntry(id)
        }
        onRevealLocationRequested: function (id) {
            if (page.backend)
                page.backend.revealLibraryEntry(id)
        }
        onDownloadRequested: function (id) {
            if (page.backend)
                page.backend.downloadEntry(id)
        }
        onShareRequested: function (entry) {
            if (entry)
                shareDialog.openFor(entry, [entry.id])
        }
        onRefreshRequested: page.refresh()
    }

    ShareDialog {
        id: shareDialog
        backend: page.backend
    }

    Connections {
        target: page.backend

        function onChanged() {
            if (!page.backend || page.backend.libraryKind !== page.kind)
                return
            if (page.lastRevision === page.backend.libraryRevision)
                return
            page.lastRevision = page.backend.libraryRevision
            if (page.entries.length === 0) {
                libraryList.currentIndex = -1
                libraryGrid.currentIndex = -1
                libraryGroupedGrid.currentIndex = -1
                page.currentEntryId = ""
            } else {
                let nextIndex = page.indexOfCurrentEntry()
                if (nextIndex < 0) {
                    nextIndex = Math.max(0, Math.min(
                                             page.activeView.currentIndex,
                                             page.entries.length - 1))
                }
                page.selectIndex(nextIndex, false)
            }
            if (page.visible)
                Qt.callLater(page.focusActiveView)
        }
    }
}
