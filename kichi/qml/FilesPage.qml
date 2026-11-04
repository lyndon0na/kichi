import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 文件页（S3）：面包屑导航 + 目录列表。
// 只调 backend 的 Q_INVOKABLE；配额只在侧栏账号区里显示（AccountCard.qml），这里不重复。
// 播放（S5）：右键菜单点「播放」→ 立刻弹固定的清晰度菜单（原画 / 1080P / 720P / 480P），
// 选中某一档后才去请求直链。
// 搜索（S9）：搜索框在标题行（main.qml 里的 FilesPageSearchBar），输入即搜全盘
// （优先本地索引；未就绪时只筛当前目录）；结果直接复用当前列表 / 网格视图。
Item {
    id: page

    property var backend: null
    // 清晰度菜单全应用只保留一个可见实例：文件页和离线任务页播放时共用它，
    // 资料库页使用自己的菜单。否则隐藏页面也会响应同一个 playMenu 信号。
    property bool playMenuEnabled: true
    // 多选状态（纯逻辑在 FileSelection.qml）
    FileSelection {
        id: selection
    }

    // 搜索视图开着：同一套列表 / 网格切换 model 显示结果
    readonly property bool searching: page.backend !== null && page.backend.searchActive
    // 目录与搜索结果共用当前显示方式
    readonly property bool gridMode: page.backend !== null
                                     && page.backend.contentView === "grid"
    readonly property bool canDropUploads: page.backend !== null
                                           && page.backend.authState === "signedIn"
                                           && !page.searching
                                           && !page.backend.filesLoading
    readonly property string uploadDestinationName: {
        const crumbs = page.backend && page.backend.breadcrumb
                     ? page.backend.breadcrumb : []
        const parts = []
        for (let i = 0; i < crumbs.length; i++)
            parts.push(String(crumbs[i]))
        return parts.length > 0 ? parts.join(" / ") : qsTr("全部文件")
    }
    // 从搜索结果进目录时，Rust 会先退出搜索、再异步读目标目录。这里保留
    // 点击瞬间的结果快照，等 filesLoading 结束再换 model，避免闪回原目录。
    property var heldSearchEntries: []
    property string heldSearchStatus: ""
    property bool heldSearchTruncated: false
    property bool holdingSearchEntries: false
    /// 主窗口是否显示；缩到托盘时页面本身仍可见，但不能为后台任务刷新。
    property bool windowVisible: true
    /// 离线任务完成后暂不能立即刷新的目录；null 表示没有待处理目录。
    property var pendingOfflineTaskFolder: null
    readonly property bool showingSearchEntries: page.searching
                                                  || page.holdingSearchEntries
    readonly property var displayEntries: {
        if (!page.backend)
            return []
        if (page.holdingSearchEntries)
            return page.heldSearchEntries
        return page.searching ? page.backend.searchResults : page.backend.files
    }
    readonly property bool backendSessionActive: page.backend === null
                                                 || typeof page.backend.authState === "undefined"
                                                 || page.backend.authState === "signedIn"
    readonly property bool showLoadingSkeleton: loadingSkeletonGate.showSkeleton
    readonly property var fileRows: entryRows.model
    readonly property var activeFileView: page.gridMode ? fileGrid : fileList
    readonly property string playLoadingId: {
        if (!page.backend || typeof page.backend.playLoadingId !== "string")
            return ""
        return page.backend.playLoadingId
    }
    readonly property var playingIds: {
        if (!page.backend || typeof page.backend.playingIds === "undefined")
            return []
        return page.backend.playingIds || []
    }

    function isEntryPlayLoading(id) {
        return page.backend !== null
                && page.backend.playLoading === true
                && page.playLoadingId === String(id)
    }

    function isEntryPlaying(id) {
        const target = String(id)
        for (let i = 0; i < page.playingIds.length; i++) {
            if (String(page.playingIds[i]) === target)
                return true
        }
        return false
    }

    function shareNamesFor(ids) {
        const names = []
        for (let i = 0; i < (ids ? ids.length : 0); i++) {
            const entry = interaction.entryForId(ids[i])
            names.push(entry && entry.name ? String(entry.name) : "")
        }
        return names
    }

    /// 搜索结果第二行的位置说明；回收站行直接显示「回收站 · 原位置」。
    function searchDetailText(entry) {
        if (!entry)
            return ""
        if (entry.inTrash === true)
            return String(entry.folderText || qsTr("回收站"))
        return qsTr("所在文件夹：%1").arg(entry.folderText
                                          || qsTr("（所在文件夹）"))
    }

    function gridDetailText(entry) {
        if (!entry)
            return ""
        if (page.showingSearchEntries)
            return page.searchDetailText(entry)
        const modified = String(entry.modifiedText || "")
        if (entry.kind === "folder")
            return modified.length > 0 ? qsTr("文件夹 · %1").arg(modified)
                                       : qsTr("文件夹")
        const size = String(entry.sizeText || qsTr("—"))
        return modified.length > 0 ? qsTr("%1 · %2").arg(size).arg(modified)
                                   : size
    }

    // 滚动位置记忆 / 导航 / 快捷键 / 操作完成后的收尾（纯逻辑在 FilePageInteraction.qml）
    FilePageInteraction {
        id: interaction
        backend: page.backend
        selection: selection
        fileList: page.activeFileView
        menu: menu
        flashTimer: createdFlashTimer
        entries: page.displayEntries
        searching: page.showingSearchEntries
        navigationLocked: page.holdingSearchEntries
        viewMode: page.gridMode ? "grid" : "list"
        onFolderOpenRequested: function (id) {
            page.openEntryFolder(id)
        }
    }

    Timer {
        id: createdFlashTimer
        interval: 1400
        repeat: false
        onTriggered: interaction.clearCreatedFlash()
    }

    // 列表密度（行高 / 字号 / 图标 / 列宽）都在 FileListDensity 里，
    // 表头、行、搜索结果共用同一份数值
    FileListDensity {
        id: density
        objectName: "fileListDensity"
    }

    // Rust 发布新数组时只做行级差异，切页 / 后台校准都不会重灌列表。
    EntryRows {
        id: entryRows
        source: page.displayEntries
    }

    LoadingSkeletonGate {
        id: loadingSkeletonGate

        objectName: "filesLoadingSkeletonGate"
        active: page.visible && !page.showingSearchEntries
        sessionActive: page.backendSessionActive
        loading: page.backend !== null && page.backend.filesLoading
        // filesLoading 只在新目录没有快照时置位；上一目录的条目仍留在
        // 模型里，但不算目标目录已有内容，必须让骨架盖住它。
        count: page.backend !== null && page.backend.filesLoading
               ? 0 : page.displayEntries.length
    }

    // 页面尺寸常量（滚动条车道 10px，见 KScrollBar / docs/pitfalls.md 第 13 条）
    UiMetrics {
        id: metrics
    }

    readonly property alias gridSideMargin: fileGrid.sideMargin
    readonly property alias gridRightMargin: fileGrid.rightMargin
    readonly property alias gridAvailableWidth: fileGrid.availableWidth
    readonly property alias gridColumns: fileGrid.columns
    readonly property alias gridCellSlotWidth: fileGrid.cellSlotWidth
    readonly property alias gridItemWidth: fileGrid.itemWidth
    readonly property alias gridItemHeight: fileGrid.itemHeight
    readonly property alias gridCellHeight: fileGrid.cellHeight

    // 面包屑在页头（main.qml）里，不是本页的一行：这里只提供跳转入口。
    // 以前本页顶部还有一行「↑ 全部文件」，和页头各成一条，看着是几块拼的。
    function goUp() {
        interaction.goParent()
    }

    function goCrumb(index) {
        interaction.goCrumb(index)
    }

    // —— 搜索入口 ——
    //
    // 搜索框本身在标题行（FilesPageSearchBar.qml）：输入即搜全盘（本地索引优先），
    // 回车 / 内嵌按钮再跑一次。本页只负责显示结果和从结果跳转。

    // 标题行的「＋ 新建文件夹」调这里：弹窗和菜单都在本页（FilesPageMenu）
    function beginNewFolder() {
        menu.openNewFolder()
    }

    // 标题行的刷新按钮（main.qml）调这里：先记滚动位置，回来不跳顶部
    function refreshFiles() {
        interaction.refreshFiles()
    }

    /// 离线任务页播放时复用同一张清晰度菜单，只把任务行给的窗口坐标交给它。
    function preparePlayMenu(anchor) {
        playMenu.anchorPoint = anchor
    }

    /// 离线任务定位到条目：等父目录模型显示后选中并滚动到目标。
    function revealEntryById(id) {
        interaction.revealEntryById(id)
    }

    // 页头的视图列表调这里：先记当前滚动位置，再让目标视图恢复自己的位置。
    function setContentView(view) {
        if (!page.backend || typeof page.backend.setContentView !== "function")
            return
        const next = view === "grid" ? "grid" : "list"
        if (page.backend.contentView === next)
            return
        // 搜索结果是临时模型，不写进目录的列表 / 网格滚动记忆。
        if (page.showingSearchEntries) {
            page.backend.setContentView(next)
            return
        }
        interaction.rememberScroll()
        page.backend.setContentView(next)
    }

    function focusActiveView() {
        if (page.activeFileView && page.visible && !page.holdingSearchEntries)
            page.activeFileView.forceActiveFocus()
    }

    function clearHeldSearchEntries() {
        page.holdingSearchEntries = false
        page.heldSearchEntries = []
        page.heldSearchStatus = ""
        page.heldSearchTruncated = false
    }

    function refreshPendingOfflineTaskFolder() {
        if (page.pendingOfflineTaskFolder === null || !page.backend)
            return
        if (!page.visible || !page.windowVisible || page.showingSearchEntries)
            return
        const folderId = String(page.pendingOfflineTaskFolder)
        if (String(page.backend.currentDirId) !== folderId) {
            page.pendingOfflineTaskFolder = null
            return
        }
        page.pendingOfflineTaskFolder = null
        if (typeof page.backend.refreshFilesBackground === "function")
            page.backend.refreshFilesBackground()
    }

    function holdSearchEntriesUntilLoaded(entries, statusText, truncated) {
        if (!page.backend || !page.backend.filesLoading)
            return
        page.heldSearchEntries = entries
        page.heldSearchStatus = statusText
        page.heldSearchTruncated = truncated
        page.holdingSearchEntries = true
    }

    function openEntryFolder(id) {
        if (!page.backend || page.holdingSearchEntries)
            return
        if (page.searching) {
            const entries = page.displayEntries.slice()
            const statusText = page.backend.searchStatus
            const truncated = page.backend.searchTruncated === true
            page.backend.searchOpenFolder(id)
            page.holdSearchEntriesUntilLoaded(entries, statusText, truncated)
        } else {
            interaction.navigateToDir(id)
        }
    }

    function revealSearchFolder(entry) {
        if (!page.backend || !entry || page.holdingSearchEntries)
            return
        const fromSearch = page.searching
        const entries = fromSearch ? page.displayEntries.slice() : []
        const statusText = fromSearch ? page.backend.searchStatus : ""
        const truncated = fromSearch && page.backend.searchTruncated === true
        // 文件夹结果也传它自己的父目录 id，不能再进入结果文件夹本身。
        page.revealEntryById(entry.id)
        page.backend.searchRevealFolder(String(entry.folderId || ""))
        if (fromSearch)
            page.holdSearchEntriesUntilLoaded(entries, statusText, truncated)
    }

    // 切到别的页时把搜索收掉：回来看到的是当前目录，而不是忘在一边的旧结果
    // （搜索框里的字由 FilesPageSearchBar 自己跟着 searchActive 清）
    onVisibleChanged: {
        if (!page.visible) {
            interaction.clearCreatedFlash()
            interaction.rememberScroll()
            page.clearHeldSearchEntries()
            if (page.searching && page.backend)
                page.backend.exitSearch()
            return
        }
        if (page.backend) {
            page.refreshPendingOfflineTaskFolder()
            if (typeof page.backend.ensureFilesFresh === "function")
                page.backend.ensureFilesFresh()
            else if (page.backend.filesStale)
                page.backend.refreshFiles()
        }
        // 从登录页 / 其它页切回来：主动把焦点交还给列表，否则快捷键收不到。
        // fileList 可能还没构造（初次可见时），判空避免无谓的 TypeError。
        page.focusActiveView()
        Qt.callLater(function () {
            interaction.restoreScrollNow()
            interaction.tryRevealPending()
        })
    }

    onGridModeChanged: {
        interaction.prepareViewSwitch(page.gridMode ? "grid" : "list")
        Qt.callLater(interaction.restoreScrollNow)
        Qt.callLater(page.focusActiveView)
        Qt.callLater(interaction.tryRevealPending)
    }
    onSearchingChanged: {
        selection.clear()
        if (page.searching)
            page.clearHeldSearchEntries()
        else
            Qt.callLater(page.refreshPendingOfflineTaskFolder)
    }
    onHoldingSearchEntriesChanged: {
        if (!page.holdingSearchEntries)
            Qt.callLater(page.refreshPendingOfflineTaskFolder)
    }
    onWindowVisibleChanged: {
        if (page.windowVisible)
            page.refreshPendingOfflineTaskFolder()
    }
    onDisplayEntriesChanged: {
        // 乐观移除 / 回滚期间保留选择，失败恢复后才能仍然指向原来的条目；
        // 操作成功由 handleFileActionDone 清掉移走 / 回收站的选择。
        if (page.backend)
            selection.pruneExcept(page.displayEntries, page.backend.fileActionIds)
        Qt.callLater(interaction.tryRevealPending)
    }

    ColumnLayout {
        anchors.fill: parent
        // 与传输 / 离线 / 设置页同一个上边距：换页时内容不在纵向上跳一下
        anchors.topMargin: metrics.pageMargin
        spacing: 8

        SearchStatusBar {
            objectName: "searchStatusBar"
            visible: page.showingSearchEntries
            Layout.fillWidth: true
            backend: page.showingSearchEntries ? page.backend : null
            statusText: page.holdingSearchEntries
                        ? page.heldSearchStatus
                        : (page.backend ? page.backend.searchStatus : "")
            truncated: page.holdingSearchEntries
                       ? page.heldSearchTruncated
                       : (page.backend !== null
                          && page.backend.searchTruncated === true)
        }

        // —— 列表（当前目录 / 搜索结果共用这块区域）——
        // 不用 Frame：它的边框画在外层，会被盖住的上下两条边吃掉，
        // 只剩左右两段淡线，看起来像对不齐的框。列表直接铺在背景上。
        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true

            ColumnLayout {
                anchors.fill: parent
                spacing: 4

                // 目录与搜索结果共用同一张列表和表头。
                FileListHeader {
                    visible: !page.gridMode
                    Layout.fillWidth: true
                    backend: page.backend
                    thumbnailSlotWidth: density.thumbnailSlotWidth
                    sizeColumnWidth: density.sizeColumnWidth
                    modifiedColumnWidth: density.modifiedColumnWidth
                    contentPadding: density.rowPadding
                    columnSpacing: density.columnSpacing
                    headerRowHeight: density.headerRowHeight
                    headerPixelSize: density.metaPixelSize
                }

                Item {
                    Layout.fillWidth: true
                    Layout.fillHeight: true

                    ListView {
                        id: fileList
                        objectName: "fileList"

                        anchors.fill: parent
                        clip: true
                        // 文件页是平铺表格：行和表头共用 34 / 48 高度，
                        // 只靠 1px 底部分隔线区分，不再用行间缝隙。
                        spacing: 0
                        visible: !page.gridMode
                        model: page.fileRows
                        boundsBehavior: Flickable.StopAtBounds
                        // 列表铺满到页面右缘，条画在右侧 10px 车道里；
                        // 委托宽度减去车道，文字永远停在条左边
                        ScrollBar.vertical: KScrollBar {}

                        // 快捷键要有焦点才收得到（KDE Wayland 下窗口级 Keys 不可靠）。
                        //
                        // ⚠ 只在 Component.onCompleted 抢一次焦点是不够的：本页在登录页
                        // 之后才可见，创建时拿不到 activeFocus，登录进来后 Qt 也不会自动
                        // 把焦点交回来（表现就是所有快捷键都没反应）。所以「变成可见」
                        // 时也要抢一次；从搜索视图切回来同样走这里。
                        focus: true
                        Component.onCompleted: forceActiveFocus()
                        onVisibleChanged: {
                            if (visible)
                                forceActiveFocus()
                        }
                        Keys.onPressed: function (event) {
                            interaction.handleKey(event)
                        }

                        // 布局完成后再恢复滚动位置（模型更新时 contentHeight 是逐步算出来的）
                        onContentHeightChanged: {
                            if (!interaction.tryRevealPending()
                                    && !interaction.tryRevealCreated())
                                interaction.restoreScrollNow()
                        }

                        // 空白处右键：行自己用 MouseArea 处理右键，这里只接住没命中行的位置。
                        // TapHandler 的 position 以内容区为原点、纵向随 contentY 走
                        // （docs/pitfalls.md 第 44 条），先换算回视口坐标。
                        TapHandler {
                            acceptedButtons: Qt.RightButton
                            onTapped: function (point) {
                                if (!page.backend || page.showingSearchEntries)
                                    return
                                const viewport = fileList.mapFromItem(
                                    null, point.scenePosition.x,
                                    point.scenePosition.y)
                                if (fileList.indexAt(viewport.x,
                                                     viewport.y + fileList.contentY) >= 0)
                                    return
                                menu.openBlankAt(fileList.mapToItem(
                                                     null, viewport.x, viewport.y))
                            }
                        }

                        delegate: FileRow {
                            id: row
                            width: ListView.view.width - metrics.scrollBarSlot
                            backend: page.backend
                            rowIndex: index
                            thumbnailSlotWidth: density.thumbnailSlotWidth
                            thumbnailSlotHeight: density.thumbnailSlotHeight
                            sizeColumnWidth: density.sizeColumnWidth
                            modifiedColumnWidth: density.modifiedColumnWidth
                            columnSpacing: density.columnSpacing
                            contentPadding: density.rowPadding
                            rowHeight: density.rowHeight
                            textPixelSize: density.namePixelSize
                            metaPixelSize: density.metaPixelSize
                            iconPixelSize: density.iconPixelSize
                            videoBadgeSize: density.listVideoBadgeSize
                            videoBadgeMargin: density.listVideoBadgeMargin
                            playLoading: page.isEntryPlayLoading(entry.id)
                            playing: page.isEntryPlaying(entry.id)
                            createdFlash: !!entry
                                          && entry.id === interaction.createdFlashId
                            detailText: page.showingSearchEntries
                                        ? page.searchDetailText(entry)
                                        : ""
                            // 回收站里的位置不是可进入的现役层级
                            detailClickable: page.showingSearchEntries
                                             && !!entry.folderId
                                             && entry.inTrash !== true
                            // 选中态按 id 判断：刷新 / 排序后仍指向同一条目
                            highlighted: !!entry && selection.isSelected(entry.id)
                            // 单击：把修饰键交给 FileSelection 处理（Ctrl 切换 / Shift 区间）
                            onSelectRequested: function (modifiers, index) {
                                const entries = page.displayEntries
                                if (index < 0 || index >= entries.length)
                                    return
                                selection.click(entries[index].id, index, modifiers, entries)
                            }
                            // 双击文件夹：交给页面先记滚动位置再进目录
                            onFolderOpenRequested: function (id) {
                                page.openEntryFolder(id)
                            }
                            onDetailRequested: {
                                page.revealSearchFolder(entry)
                            }
                            // 双击媒体文件：把行内的点换算成窗口坐标交给清晰度菜单。
                            // ⚠ 必须用赋值，不能在上面写 anchorPoint: page.playAnchor 的绑定：
                            // 给有绑定的属性赋值会永久断掉绑定，菜单就永远停在第一次的位置
                            onPlayRequested: function (id, point) {
                                playMenu.anchorPoint = row.mapToItem(null, point.x, point.y)
                                if (page.backend)
                                    page.backend.requestPlay(id)
                            }
                            // FileRow 给的是行内坐标：换算成窗口坐标再交给右键菜单。
                            // 右键未选中的行时先收窄成这一条（文件管理器的惯例）
                            onContextMenu: function (point) {
                                selection.ensureSelected(entry)
                                fileList.forceActiveFocus()
                                var windowPoint = row.mapToItem(null, point.x, point.y)
                                menu.openContext(entry, windowPoint, selection.idList(),
                                                 page.searching)
                            }
                        }
                    }

                    ScrollFirstClickArea {
                        id: fileListFirstClick
                        view: fileList
                        scrollBarSlot: metrics.scrollBarSlot

                        onClickedAt: function (index, modifiers, position) {
                            const item = fileList.itemAtIndex(index)
                            if (!item)
                                return
                            fileList.forceActiveFocus()
                            fileListFirstClick.invokeFirstClick(
                                item, index, modifiers, position.x, position.y)
                        }
                    }
                }
            }

            // 网格视图：和列表共用同一块内容区、同一个 model 和同一套选择逻辑。
            ContentGridView {
                id: fileGrid

                anchors.fill: parent
                visible: page.gridMode
                model: page.fileRows
                density: density
                metrics: metrics
                reserveDetail: true
                viewObjectName: "fileGrid"

                onKeyPressed: function (event) {
                    interaction.handleKey(event)
                }
                onContentHeightChanged: {
                    if (!interaction.tryRevealPending()
                            && !interaction.tryRevealCreated())
                        interaction.restoreScrollNow()
                }
                onBlankContextMenuRequested: function (point) {
                    if (!page.backend || page.showingSearchEntries)
                        return
                    menu.openBlankAt(fileGrid.mapToItem(null, point.x, point.y))
                }

                delegate: Component {
                    FileGridItem {
                        id: gridItem

                        width: fileGrid.itemWidth
                        height: fileGrid.itemHeight
                        backend: page.backend
                        rowIndex: index
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
                        playLoading: page.isEntryPlayLoading(entry.id)
                        playing: page.isEntryPlaying(entry.id)
                        selected: !!entry && selection.isSelected(entry.id)
                        createdFlash: !!entry
                                      && entry.id === interaction.createdFlashId
                        detailText: page.gridDetailText(entry)
                        detailClickable: page.showingSearchEntries
                                         && !!entry.folderId
                                         && entry.inTrash !== true

                        onSelectRequested: function (modifiers, itemIndex) {
                            const entries = page.displayEntries
                            if (itemIndex < 0 || itemIndex >= entries.length)
                                return
                            fileGrid.currentIndex = itemIndex
                            selection.click(entries[itemIndex].id, itemIndex,
                                            modifiers, entries)
                        }
                        onFolderOpenRequested: function (id) {
                            page.openEntryFolder(id)
                        }
                        onDetailRequested: {
                            page.revealSearchFolder(entry)
                        }
                        onPlayRequested: function (id, point) {
                            playMenu.anchorPoint = gridItem.mapToItem(
                                null, point.x, point.y)
                            if (page.backend)
                                page.backend.requestPlay(id)
                        }
                        onContextMenu: function (point) {
                            selection.ensureSelected(entry)
                            fileGrid.forceActiveFocus()
                            var windowPoint = gridItem.mapToItem(
                                null, point.x, point.y)
                            menu.openContext(entry, windowPoint, selection.idList(),
                                             page.searching)
                        }
                    }
                }
            }

            LoadingSkeleton {
                anchors.fill: parent
                visible: page.showLoadingSkeleton
                gridMode: page.gridMode
                density: density
                metrics: metrics
                gridView: page.gridMode ? fileGrid : null
                listTopInset: page.gridMode ? 0 : density.headerRowHeight + 4
                trailingColumnWidths: [density.sizeColumnWidth,
                                       density.modifiedColumnWidth]
            }

            // 居中对齐到列表区域：表头在时提示落在列表中间，不会压在表头上
            Label {
                anchors.centerIn: parent
                text: qsTr("这个文件夹是空的")
                color: palette.placeholderText
                visible: !page.showingSearchEntries && page.backend !== null
                         && !page.backend.filesLoading
                         && !page.showLoadingSkeleton
                         && page.backend.files.length === 0
            }

            SearchEmptyState {
                anchors.centerIn: parent
                width: Math.min(parent.width - 40, 460)
                backend: page.showingSearchEntries ? page.backend : null
            }

            SearchNavigationOverlay {
                anchors.fill: parent
                running: page.holdingSearchEntries
            }
        }
    }

    FilesPageUploadDropArea {
        anchors.fill: parent
        z: 40
        backend: page.backend
        acceptsDrops: page.canDropUploads
        parentId: page.backend ? page.backend.currentDirId : ""
        destinationName: page.uploadDestinationName
    }

    // —— 右键菜单与写操作弹窗（内容在 FilesPageMenu.qml / FileActionDialogs.qml）——
    FilesPageMenu {
        id: menu
        anchors.fill: parent
        backend: page.backend
        onPlayRequested: function (anchor) {
            // 右键菜单给的已经是窗口坐标，直接当清晰度菜单的锚点
            playMenu.anchorPoint = anchor
        }
        onMoveRequested: function (entry, ids) {
            movePicker.openForMove(entry, ids, interaction.folderIdsFor(ids))
        }
        onCopyRequested: function (entry, ids) {
            movePicker.openForCopy(entry, ids, interaction.folderIdsFor(ids))
        }
        onShareRequested: function (entry, ids) {
            shareDialog.openFor(entry, ids, page.shareNamesFor(ids))
        }
        onRefreshRequested: interaction.refreshFiles()
    }

    // —— 移动 / 复制目标选择器 ——
    MovePickerDialog {
        id: movePicker
        backend: page.backend
    }

    ShareDialog {
        id: shareDialog
        backend: page.backend
    }

    // —— 列表刷新 / 文件操作完成：恢复滚动、清理选择 ——
    Connections {
        target: page.backend

        function onChanged() {
            interaction.noteFilesRevision()
            if (page.holdingSearchEntries && page.backend
                    && !page.backend.filesLoading) {
                page.clearHeldSearchEntries()
            }
        }

        function onFileActionDone(action, ok, message, ids) {
            interaction.handleFileActionDone(action, ok, ids)
            // 搜索里还原成功后重查：该行必须失去「回收站」标注
            if (action === "untrash" && ok && page.searching && page.backend
                    && typeof page.backend.searchLocal === "function")
                page.backend.searchLocal(page.backend.searchQuery)
        }

        function onFolderCreateStarted(id) {
            if (page.visible && !page.showingSearchEntries)
                interaction.flashCreatedEntry(id)
        }

        function onFolderCreated(id) {
            if (page.visible && !page.showingSearchEntries)
                interaction.resolveCreatedEntry(id)
        }

        function onOfflineTaskFolderChanged(folderId) {
            if (!page.backend)
                return
            if (String(page.backend.currentDirId) !== String(folderId))
                return
            page.pendingOfflineTaskFolder = String(folderId)
            page.refreshPendingOfflineTaskFolder()
        }
    }

    // —— 清晰度菜单（内容在 FilesPagePlayMenu.qml）——
    // 锚点由上面的点击处直接赋值（窗口坐标），这里不要再写 anchorPoint 绑定
    FilesPagePlayMenu {
        id: playMenu
        backend: page.playMenuEnabled ? page.backend : null
    }
}
