import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 文件页（S3）：面包屑导航 + 目录列表。
// 只调 backend 的 Q_INVOKABLE；配额只在侧栏账号卡里显示（SideBar.qml），这里不重复。
// 播放（S5）：右键菜单点「播放」→ 立刻弹固定的清晰度菜单（原画 / 1080P / 720P / 480P），
// 选中某一档后才去请求直链。
// 搜索（S9）：搜索框在标题行（main.qml 里的 FilesPageSearchBar），输入只筛当前目录，
// 全盘搜索由回车 / 内嵌按钮触发；结果在 SearchResults.qml 里，和目录列表共用同一块列表区。
Item {
    id: page

    property var backend: null
    // 多选状态（纯逻辑在 FileSelection.qml）
    FileSelection {
        id: selection
    }

    // 搜索视图开着：列表区换成结果
    readonly property bool searching: page.backend !== null && page.backend.searchActive
    // 文件页当前显示方式；搜索视图始终使用独立的结果列表
    readonly property bool gridMode: page.backend !== null
                                     && page.backend.fileView === "grid"
    readonly property var activeFileView: page.gridMode ? fileGrid : fileList

    // 滚动位置记忆 / 导航 / 快捷键 / 操作完成后的收尾（纯逻辑在 FilePageInteraction.qml）
    FilePageInteraction {
        id: interaction
        backend: page.backend
        selection: selection
        fileList: page.activeFileView
        menu: menu
        searching: page.searching
        viewMode: page.gridMode ? "grid" : "list"
    }

    // 列表密度（行高 / 字号 / 图标 / 列宽）都在 FileListDensity 里，
    // 表头、行、搜索结果共用同一份数值
    FileListDensity {
        id: density
        objectName: "fileListDensity"
    }

    // 页面尺寸常量（滚动条车道 10px，见 KScrollBar / docs/pitfalls.md 第 13 条）
    UiMetrics {
        id: metrics
    }

    // 网格列宽：左右各留一份页面内边距，右侧再让出滚动条车道。
    // GridView 按 cellWidth 向下取整列数，所以 cellWidth 只能等于可用宽度 / 列数；
    // 若把列间距也算进去，会少排一列，在右侧留下整列空白。
    readonly property int gridSideMargin: metrics.pageMargin
    readonly property int gridRightMargin: metrics.scrollBarSlot + gridSideMargin
    readonly property real gridAvailableWidth: Math.max(
        0, fileGrid.width - gridSideMargin - gridRightMargin)
    readonly property int gridColumns: Math.max(1, Math.floor(
        (gridAvailableWidth + density.gridColumnGap)
        / (density.gridMinItemWidth + density.gridColumnGap)))
    readonly property int gridCellSlotWidth: gridColumns > 0
        ? Math.max(1, Math.floor(gridAvailableWidth / gridColumns)) : 0
    readonly property real gridItemWidth: Math.max(1, gridCellSlotWidth
                                                     - density.gridColumnGap)
    readonly property real gridItemHeight: density.gridItemHeight(gridItemWidth)
    readonly property real gridCellHeight: gridItemHeight + density.gridRowGap

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
    // 搜索框本身在标题行（FilesPageSearchBar.qml）：输入只筛本目录，
    // 回车 / 内嵌按钮才全盘搜。本页只负责显示结果和从结果跳转。

    // 标题行的「＋ 新建文件夹」调这里：弹窗和菜单都在本页（FilesPageMenu）
    function beginNewFolder() {
        menu.openNewFolder()
    }

    // 标题行的刷新按钮（main.qml）调这里：先记滚动位置，回来不跳顶部
    function refreshFiles() {
        interaction.refreshFiles()
    }

    // 页头的视图列表调这里：先记当前滚动位置，再让目标视图恢复自己的位置。
    function setFileView(view) {
        if (!page.backend || typeof page.backend.setFileView !== "function")
            return
        const next = view === "grid" ? "grid" : "list"
        if (page.backend.fileView === next)
            return
        interaction.rememberScroll()
        interaction.prepareViewSwitch(next)
        page.backend.setFileView(next)
    }

    function focusActiveView() {
        if (page.activeFileView && page.visible && !page.searching)
            page.activeFileView.forceActiveFocus()
    }

    // 搜索结果里的播放 / 下载也在右键菜单里，锚点用右键位置
    function openResultContext(entry, point) {
        // 搜索结果给的是本视图坐标：换算成窗口坐标再交给右键菜单（绝对定位）
        var windowPoint = searchResults.mapToItem(null, point.x, point.y)
        menu.openContext(entry, windowPoint, [entry.id], true)
    }

    // 切到别的页时把搜索收掉：回来看到的是当前目录，而不是忘在一边的旧结果
    // （搜索框里的字由 FilesPageSearchBar 自己跟着 searchActive 清）
    onVisibleChanged: {
        if (!page.visible) {
            if (page.searching && page.backend)
                page.backend.exitSearch()
            return
        }
        if (page.backend && page.backend.filesStale)
            page.backend.refreshFiles()
        // 从登录页 / 其它页切回来：主动把焦点交还给列表，否则快捷键收不到。
        // fileList 可能还没构造（初次可见时），判空避免无谓的 TypeError。
        page.focusActiveView()
    }

    onGridModeChanged: Qt.callLater(page.focusActiveView)

    ColumnLayout {
        anchors.fill: parent
        // 与传输 / 离线 / 设置页同一个上边距：换页时内容不在纵向上跳一下
        anchors.topMargin: metrics.pageMargin
        spacing: 8

        // —— 列表（当前目录 / 搜索结果共用这块区域）——
        // 不用 Frame：它的边框画在外层，会被盖住的上下两条边吃掉，
        // 只剩左右两段淡线，看起来像对不齐的框。列表直接铺在背景上。
        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true

            ColumnLayout {
                anchors.fill: parent
                spacing: 4

                // 表头只属于当前目录视图：搜索结果列不同（多类型列、所在文件夹），
                // 不硬套同一套列宽。排序箭头读的是 backend 的排序状态。
                FileListHeader {
                    visible: !page.searching && !page.gridMode
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

                ListView {
                    id: fileList
                    objectName: "fileList"

                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    clip: true
                    // 文件页是平铺表格：行和表头共用 34 / 48 高度，
                    // 只靠 1px 底部分隔线区分，不再用行间缝隙。
                    spacing: 0
                    // 搜索视图显示结果；否则显示当前目录
                    visible: !page.searching && !page.gridMode
                    model: page.backend ? page.backend.files : []
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
                    onContentHeightChanged: interaction.restoreScrollNow()

                    // 空白处右键：行自己用 MouseArea 处理右键，这里只接住没命中行的位置。
                    // 坐标要换成内容坐标（加上 contentY）才能用 indexAt。
                    TapHandler {
                        acceptedButtons: Qt.RightButton
                        onTapped: function (point) {
                            if (!page.backend)
                                return
                            const x = point.position.x
                            const y = point.position.y
                            if (fileList.indexAt(x, y + fileList.contentY) >= 0)
                                return
                            menu.openBlankAt(fileList.mapToItem(null, x, y))
                        }
                    }

                    delegate: FileRow {
                        id: row
                        width: ListView.view.width - metrics.scrollBarSlot
                        entry: modelData
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
                        // 选中态按 id 判断：刷新 / 排序后仍指向同一条目
                        highlighted: !!entry && selection.isSelected(entry.id)
                        // 单击：把修饰键交给 FileSelection 处理（Ctrl 切换 / Shift 区间）
                        onSelectRequested: function (modifiers, index) {
                            const entries = page.backend ? page.backend.files : []
                            if (index < 0 || index >= entries.length)
                                return
                            selection.click(entries[index].id, index, modifiers, entries)
                        }
                        // 双击文件夹：交给页面先记滚动位置再进目录
                        onFolderOpenRequested: function (id) {
                            interaction.navigateToDir(id)
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
                            menu.openContext(entry, windowPoint, selection.idList(), false)
                        }
                    }
                }
            }

            // 网格视图：和列表共用同一块内容区、同一个 model 和同一套选择逻辑。
            // GridView 的 cellWidth 含列间空隙，委托宽度由 FilesPage 统一算好，
            // 这样最后一列不会钻到滚动条车道下面。
            GridView {
                id: fileGrid
                objectName: "fileGrid"

                anchors.fill: parent
                visible: !page.searching && page.gridMode
                model: page.backend ? page.backend.files : []
                cellWidth: page.gridCellSlotWidth
                cellHeight: page.gridCellHeight
                leftMargin: page.gridSideMargin
                rightMargin: page.gridRightMargin
                topMargin: 0
                bottomMargin: 0
                clip: true
                boundsBehavior: Flickable.StopAtBounds
                ScrollBar.vertical: KScrollBar {}

                focus: true
                Keys.onPressed: function (event) {
                    interaction.handleKey(event)
                }
                onVisibleChanged: {
                    if (visible)
                        forceActiveFocus()
                }
                onContentHeightChanged: interaction.restoreScrollNow()

                // 空白处右键：和列表一样只接住没命中条目的位置。
                TapHandler {
                    acceptedButtons: Qt.RightButton
                    onTapped: function (point) {
                        if (!page.backend)
                            return
                        const x = point.position.x
                        const y = point.position.y
                        if (fileGrid.indexAt(x, y + fileGrid.contentY) >= 0)
                            return
                        menu.openBlankAt(fileGrid.mapToItem(null, x, y))
                    }
                }

                delegate: FileGridItem {
                    id: gridItem

                    width: page.gridItemWidth
                    height: page.gridItemHeight
                    entry: modelData
                    backend: page.backend
                    rowIndex: index
                    contentPadding: density.gridCardPadding
                    cardSpacing: density.gridCardSpacing
                    nameLineHeight: density.gridNameLineHeight
                    nameBlockHeight: density.gridNameBlockHeight
                    thumbnailRadius: density.gridThumbnailRadius(width)
                    textPixelSize: density.namePixelSize
                    iconPixelSize: density.gridIconPixelSize
                    videoBadgeSize: density.gridVideoBadgeSize
                    selected: !!entry && selection.isSelected(entry.id)

                    onSelectRequested: function (modifiers, itemIndex) {
                        const entries = page.backend ? page.backend.files : []
                        if (itemIndex < 0 || itemIndex >= entries.length)
                            return
                        fileGrid.currentIndex = itemIndex
                        selection.click(entries[itemIndex].id, itemIndex, modifiers, entries)
                    }
                    onFolderOpenRequested: function (id) {
                        interaction.navigateToDir(id)
                    }
                    onPlayRequested: function (id, point) {
                        playMenu.anchorPoint = gridItem.mapToItem(null, point.x, point.y)
                        if (page.backend)
                            page.backend.requestPlay(id)
                    }
                    onContextMenu: function (point) {
                        selection.ensureSelected(entry)
                        fileGrid.forceActiveFocus()
                        var windowPoint = gridItem.mapToItem(null, point.x, point.y)
                        menu.openContext(entry, windowPoint, selection.idList(), false)
                    }
                }
            }

            // 居中对齐到列表区域：表头在时提示落在列表中间，不会压在表头上
            Label {
                anchors.centerIn: parent
                text: qsTr("这个文件夹是空的")
                color: palette.placeholderText
                visible: !page.searching && page.backend !== null && !page.backend.filesLoading
                         && page.backend.files.length === 0
            }

            // 搜索结果（全盘搜索时才出现，盖住同一块区域）
            SearchResults {
                id: searchResults
                anchors.fill: parent
                // 必须真隐藏：它内部的 ListView（Flickable）会吃掉鼠标按压。
                // 只把 backend 置空、不设 visible 时，它透明地盖在目录列表上，
                // 状态行以下的区域全被它截走——表现就是「只有第一行能点」
                visible: page.searching
                backend: page.searching ? page.backend : null
                // 行高 / 字号与目录列表同一套（结果行是两行，行高略高）
                rowHeight: density.resultRowHeight
                textPixelSize: density.namePixelSize
                metaPixelSize: density.metaPixelSize
                iconPixelSize: density.iconPixelSize
                sizeColumnWidth: density.resultSizeColumnWidth
                modifiedColumnWidth: density.resultModifiedColumnWidth

                onContextMenu: function (entry, point) {
                    page.openResultContext(entry, point)
                }
                onPlayRequested: function (id, point) {
                    // 同目录列表：同样换算成窗口坐标（见上面 FileRow 处）
                    playMenu.anchorPoint = searchResults.mapToItem(null, point.x, point.y)
                    if (page.backend)
                        page.backend.requestPlay(id)
                }
            }
        }
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
            shareDialog.openFor(entry, ids)
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
        }

        function onFileActionDone(action, ok) {
            interaction.handleFileActionDone(action, ok)
        }
    }

    // —— 清晰度菜单（内容在 FilesPagePlayMenu.qml）——
    // 锚点由上面的点击处直接赋值（窗口坐标），这里不要再写 anchorPoint 绑定
    FilesPagePlayMenu {
        id: playMenu
        backend: page.backend
    }
}
