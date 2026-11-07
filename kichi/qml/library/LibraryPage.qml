import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "../components"
import "../core"
import "../files"
import "../shares"

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
    /// 每个资料库页保留自己的快照。后端三页共用一份当前集合，切到另一页时
    /// 不能把本页模型清空，否则切回来会重新灌入整份数据，侧栏切换明显卡顿。
    property var cachedEntries: []
    property int acceptedDataRevision: -1
    readonly property var entries: page.cachedEntries
    readonly property bool loading: page.activeCollection && page.backend.libraryLoading
    readonly property bool loaded: page.activeCollection && page.backend.libraryLoaded
    readonly property string errorText: page.activeCollection ? page.backend.libraryError : ""
    readonly property bool backendSessionActive: page.backend === null
                                                 || typeof page.backend.authState === "undefined"
                                                 || page.backend.authState === "signedIn"
    readonly property bool showLoadingSkeleton: loadingSkeletonGate.showSkeleton
    property int lastRevision: -1
    property string currentEntryId: ""
    readonly property bool gridMode: page.backend !== null
                                     && page.backend.contentView === "grid"
    readonly property bool dateGrouped: page.dateSectionsEnabled
                                        && page.kind === "playback-history"
    readonly property var libraryRows: entryRows.model
    readonly property var activeView: page.gridMode
                                      ? (page.dateGrouped ? libraryGroupedGrid : libraryGrid)
                                      : activeList
    readonly property var activeList: page.dateGrouped ? libraryGroupedList : libraryList
    property bool pendingScrollRestore: false
    property real pendingScrollY: 0

    FileListDensity {
        id: density
        objectName: "libraryDensity"
    }
    EntryRows {
        id: entryRows
        // 页面离开前台后释放行模型和委托；cachedEntries 仍保留，回来可立即恢复。
        source: page.active ? page.entries : []
    }
    UiMetrics { id: metrics }
    KTheme { id: theme }

    KScrollMemory {
        id: scrollMemory
        flickable: page.activeView
    }

    LoadingSkeletonGate {
        id: loadingSkeletonGate

        active: page.active
        sessionActive: page.backendSessionActive
        loading: page.loading
        count: page.entries.length
    }

    function isPlayLoading(id) {
        return page.backend !== null
                && page.backend.playLoading === true
                && String(page.backend.playLoadingId) === String(id)
    }

    /// 加载中的条目是否就在当前资料库页的列表里。
    /// 切到别的资料库页或数据刷新后条目不再可见时，底部提示条接管取消入口。
    readonly property bool showsInlinePlayLoading: {
        if (!page.active || !page.activeCollection
                || page.backend === null || page.backend.playLoading !== true)
            return false
        const id = String(page.backend.playLoadingId || "")
        if (id.length === 0)
            return false
        const entries = page.entries
        for (let i = 0; i < entries.length; ++i) {
            if (entries[i] && String(entries[i].id) === id)
                return true
        }
        return false
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

    function ensureFresh() {
        if (!page.backend || page.kind.length === 0 || page.loading)
            return
        if (typeof page.backend.ensureLibraryFresh === "function")
            page.backend.ensureLibraryFresh(page.kind)
        else
            page.refresh()
    }

    function acceptBackendEntries() {
        if (!page.activeCollection)
            return
        const revision = page.backend.libraryDataRevision
        if (typeof revision === "number") {
            if (page.acceptedDataRevision === revision)
                return
            page.acceptedDataRevision = revision
        }
        if (page.cachedEntries !== page.backend.libraryRows)
            page.cachedEntries = page.backend.libraryRows
    }

    function rememberScroll() {
        scrollMemory.rememberKey(page.gridMode ? "grid" : "list")
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
        scrollMemory.applyTo(page.activeView, page.pendingScrollY)
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
            libraryGroupedList.currentIndex = -1
            page.currentEntryId = ""
            libraryGrid.currentIndex = -1
            libraryGroupedGrid.currentIndex = -1
            return
        }
        const next = Math.max(0, Math.min(index, entries.length - 1))
        libraryList.currentIndex = next
        libraryGroupedList.currentIndex = next
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

    /// 视口坐标 (x, y) 下命中的资料库行；日期分区头和行下空白按空白处理。
    function rowAt(x, y) {
        const list = page.activeList
        const index = list.indexAt(x, y + list.contentY)
        if (index < 0 || index >= entries.length)
            return null
        const item = list.itemAtIndex(index)
        if (!item)
            return null
        const local = list.mapToItem(item, x, y)
        return local.x >= 0 && local.y >= 0
                && local.x < item.width && local.y < item.height ? item : null
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
        page.ensureFresh()
        page.acceptBackendEntries()
        if (!page.pendingScrollRestore) {
            page.pendingScrollY = scrollMemory.valueFor(page.gridMode ? "grid" : "list", 0)
            page.pendingScrollRestore = true
        }
        Qt.callLater(function () {
            page.restoreScroll()
            page.focusActiveView()
        })
    }

    onGridModeChanged: {
        page.pendingScrollY = scrollMemory.valueFor(page.gridMode ? "grid" : "list", 0)
        page.pendingScrollRestore = true
        const index = page.indexOfCurrentEntry()
        if (index >= 0) {
            libraryList.currentIndex = index
            libraryGroupedList.currentIndex = index
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
            Layout.rightMargin: metrics.pageMargin
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

                Item {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    visible: !page.gridMode && !page.dateGrouped

                    ListView {
                        id: libraryList
                        objectName: "libraryList"

                        anchors.fill: parent
                        visible: !page.gridMode && !page.dateGrouped
                        clip: true
                        spacing: 0
                        model: page.dateGrouped ? null : page.libraryRows
                        currentIndex: -1
                        boundsBehavior: Flickable.StopAtBounds
                        ScrollBar.vertical: KScrollBar {}
                        focus: true
                        highlightFollowsCurrentItem: false
                        Keys.onPressed: function (event) {
                            page.handleKey(event)
                        }
                        onContentHeightChanged: page.restoreScroll()

                        TapHandler {
                            acceptedButtons: Qt.RightButton
                            onTapped: function (point) {
                                if (!page.backend)
                                    return
                                // position 以内容区为原点，纵向随 contentY 走
                                // 先换算回视口坐标。
                                const viewport = libraryList.mapFromItem(
                                    null, point.scenePosition.x,
                                    point.scenePosition.y)
                                if (page.rowAt(viewport.x, viewport.y))
                                    return
                                page.openBlankMenu(libraryList.mapToItem(
                                                       null, viewport.x, viewport.y))
                            }
                        }

                        delegate: LibraryRow {
                            id: row
                            width: ListView.view.width
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

                    ScrollFirstClickArea {
                        id: libraryListFirstClick
                        view: libraryList
                        scrollBarSlot: metrics.scrollBarSlot

                        onClickedAt: function (index, modifiers, position) {
                            const item = libraryList.itemAtIndex(index)
                            if (!item)
                                return
                            libraryList.forceActiveFocus()
                            libraryListFirstClick.invokeFirstClick(
                                item, index, modifiers, position.x, position.y)
                        }
                    }
                }

                LibraryGroupedListView {
                    id: libraryGroupedList

                    objectName: "libraryGroupedListView"
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    visible: !page.gridMode && page.dateGrouped
                    libraryPage: page.dateGrouped ? page : null
                    density: density
                    metrics: metrics
                    onKeyPressed: function (event) { page.handleKey(event) }
                    onContentHeightChanged: page.restoreScroll()
                }
            }

            ContentGridView {
                id: libraryGrid

                objectName: "libraryGridContainer"
                anchors.fill: parent
                visible: page.gridMode && !page.dateGrouped
                model: page.dateGrouped ? null : page.libraryRows
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

            LoadingSkeleton {
                anchors.fill: parent
                visible: page.showLoadingSkeleton
                gridMode: page.gridMode
                density: density
                metrics: metrics
                gridView: page.gridMode
                          ? (page.dateGrouped ? libraryGroupedGrid : libraryGrid) : null
                listTopInset: page.gridMode ? 0 : density.headerRowHeight + 4
                trailingColumnWidths: [density.sizeColumnWidth,
                                       density.modifiedColumnWidth]
            }

            BusyIndicator {
                objectName: "libraryLoadingIndicator"
                anchors.centerIn: parent
                visible: page.loading && page.entries.length === 0
                         && !page.showLoadingSkeleton
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
        onStarRequested: function (id, starred) {
            if (page.backend)
                page.backend.setLibraryStarred(id, starred)
        }
        onDownloadRequested: function (id) {
            if (page.backend)
                page.backend.downloadEntry(id)
        }
        onShareRequested: function (entry) {
            if (entry)
                shareDialog.openFor(entry, [entry.id], [entry.name])
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
            // 退出登录 / 换账号后后端会把集合置空。隐藏页也必须丢掉旧账号的行，
            // 否则新账号快照恢复出的 revision 可能和旧值相同而被 accept 跳过。
            if (!page.backend || page.backend.libraryKind.length === 0) {
                if (page.cachedEntries.length > 0)
                    page.cachedEntries = []
                page.acceptedDataRevision = -1
                return
            }
            if (!page.active || page.backend.libraryKind !== page.kind)
                return
            page.acceptBackendEntries()
            if (page.lastRevision === page.backend.libraryRevision)
                return
            page.lastRevision = page.backend.libraryRevision
            if (page.entries.length === 0) {
                libraryList.currentIndex = -1
                libraryGroupedList.currentIndex = -1
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

        function onFileActionDone(action, ok, message, ids) {
            if (page.active && ok)
                page.ensureFresh()
        }
    }
}
