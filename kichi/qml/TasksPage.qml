import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 离线任务页：任务表格由后台轮询自动刷新；新任务从页头按钮打开弹窗添加。
// 顶层用 Item 而非 Page：Page 自带 header/footer，放进 StackLayout 容易撑爆高度。
Item {
    id: page

    property var backend: null
    /// 页面被侧栏切到前台：首次进入立即刷新，之后由 Rust 冷却与轮询合并请求。
    property bool active: false
    property string pendingDeleteId: ""
    property string pendingDeleteName: ""

    signal openFolderRequested(string fileId)
    signal revealLocationRequested(string fileId)
    signal playRequested(string fileId, point position)

    readonly property bool tasksLoading: backend ? backend.tasksLoading : false
    readonly property bool showEmptyState: taskRows.count === 0 && !tasksLoading
    readonly property bool showLoadingState: taskRows.count === 0 && tasksLoading
    readonly property int taskCount: taskRows.count
    readonly property int tableHeaderHeight: 34
    readonly property int tableRowHeight: 48
    readonly property int thumbnailColumnWidth: 36
    readonly property int thumbnailSlotHeight: 24
    readonly property int sizeColumnWidth: 84
    readonly property int createdColumnWidth: 120
    readonly property int statusColumnWidth: 100
    readonly property int progressColumnWidth: 150
    readonly property int tableColumnSpacing: 8
    readonly property int tableContentPadding: 12
    readonly property bool showSizeColumn: width >= 560
    readonly property bool showCreatedColumn: width >= 680

    onActiveChanged: {
        if (!active)
            return
        if (page.backend && typeof page.backend.ensureTasksFresh === "function")
            page.backend.ensureTasksFresh()
        else
            page.refresh()
    }

    // 页面尺寸常量：滚动条车道（见 KScrollBar / docs/pitfalls.md 第 13 条）
    UiMetrics {
        id: metrics
    }

    KTheme { id: theme }

    // —— 列表模型 ——
    // ⚠ 不能把 `backend.tasks` 直接当 ListView.model：每次轮询都是一个新数组，
    // ListView 会整表重灌，用户滚到哪都会被甩回顶部（2026-10-28）。
    // 这里用 ListModel 就地做差异：只有增删 / 重排才动行，字段变化只打补丁。
    ListModel {
        id: taskRows
    }

    Connections {
        target: page.backend

        function onChanged() {
            page.syncRows()
        }
    }

    Component.onCompleted: syncRows()

    readonly property int activeCount: {
        var tasks = page.backend ? page.backend.tasks : []
        var n = 0
        for (var i = 0; i < tasks.length; i++) {
            if (!tasks[i].settled)
                n++
        }
        return n
    }

    ColumnLayout {
        objectName: "tasksTableLayout"
        anchors.fill: parent
        // 右边不留：列表铺到窗口右缘，滚动条贴边
        anchors.leftMargin: metrics.pageMargin
        anchors.topMargin: metrics.pageMargin
        anchors.bottomMargin: metrics.pageMargin
        spacing: theme.gapXs

        TaskTableHeader {
            Layout.fillWidth: true
            Layout.preferredHeight: page.tableHeaderHeight
            rowHeight: page.tableHeaderHeight
            thumbnailColumnWidth: page.thumbnailColumnWidth
            sizeColumnWidth: page.sizeColumnWidth
            createdColumnWidth: page.createdColumnWidth
            statusColumnWidth: page.statusColumnWidth
            progressColumnWidth: page.progressColumnWidth
            columnSpacing: page.tableColumnSpacing
            contentPadding: page.tableContentPadding
            showSize: page.showSizeColumn
            showCreated: page.showCreatedColumn
        }

        ListView {
            id: list
            objectName: "tasksList"
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            spacing: 0
            model: taskRows
            boundsBehavior: Flickable.StopAtBounds

            // 列表铺到窗口右缘，条画在右侧 10px 车道里
            ScrollBar.vertical: KScrollBar {}

            delegate: TaskRow {
                id: row

                // 行宽减去滚动条车道，右边不会压到条
                width: list.width - metrics.scrollBarSlot
                rowHeight: page.tableRowHeight
                backend: page.backend
                thumbnailColumnWidth: page.thumbnailColumnWidth
                thumbnailSlotHeight: page.thumbnailSlotHeight
                sizeColumnWidth: page.sizeColumnWidth
                createdColumnWidth: page.createdColumnWidth
                statusColumnWidth: page.statusColumnWidth
                progressColumnWidth: page.progressColumnWidth
                columnSpacing: page.tableColumnSpacing
                contentPadding: page.tableContentPadding
                showSize: page.showSizeColumn
                showCreated: page.showCreatedColumn
                playLoading: page.isTaskPlayLoading(fileId)
                playing: page.isTaskPlaying(fileId)
                folderLoading: page.isTaskFolderLoading(fileId)
                // ⚠ 这里**不要**再写 `key: model.key ?? ""` 这类绑定：TaskRow 的字段
                // 都是 required property，ListView 会按同名角色自动注入；一旦手写
                // `model.xxx` 覆盖上去，自动注入就被顶掉，而 ListModel 的 `model`
                // 对象在这里读不到这些角色 → 整行字段全空。
                onContextMenu: function (point) {
                    taskMenu.openFor(row.key, row.name,
                                     row.mapToItem(null, point.x, point.y),
                                     row.fileId, row.kind, row.phase, row.playable)
                }
                onOpenFolderRequested: function (id) {
                    page.openFolderRequested(id)
                }
                onPlayRequested: function (id, point) {
                    page.playRequested(id, row.mapToItem(null, point.x, point.y))
                }
            }

            Label {
                objectName: "tasksEmptyState"
                anchors.centerIn: parent
                visible: page.showEmptyState
                text: qsTr("暂无离线任务\n点击右上角「新任务」添加磁力链接或下载地址")
                horizontalAlignment: Text.AlignHCenter
                color: theme.textMuted
                font.pixelSize: theme.fontBody
            }

            Column {
                objectName: "tasksLoadingState"
                anchors.centerIn: parent
                width: 240
                spacing: theme.gapSm
                visible: page.showLoadingState

                BusyIndicator {
                    anchors.horizontalCenter: parent.horizontalCenter
                    width: 30
                    height: 30
                    running: parent.visible
                }

                Label {
                    width: parent.width
                    text: qsTr("正在加载离线任务…")
                    horizontalAlignment: Text.AlignHCenter
                    color: theme.textMuted
                    font.pixelSize: theme.fontBody
                }
            }
        }
    }

    function openNewTaskDialog() {
        if (page.backend)
            newTaskDialog.open()
    }

    function refresh() {
        if (page.backend && !page.tasksLoading)
            page.backend.refreshTasks()
    }

    function isTaskPlayLoading(id) {
        return page.backend !== null
                && page.backend.playLoading === true
                && String(page.backend.playLoadingId || "") === String(id)
    }

    function isTaskPlaying(id) {
        if (!page.backend || typeof page.backend.playingIds === "undefined")
            return false
        const ids = page.backend.playingIds || []
        const target = String(id)
        for (var i = 0; i < ids.length; i++) {
            if (String(ids[i]) === target)
                return true
        }
        return false
    }

    function isTaskFolderLoading(id) {
        return page.backend !== null
                && String(page.backend.taskFolderLoadingId || "") === String(id)
    }

    /// 把 Rust 推来的任务列表同步进 ListModel：增删 / 移位 / 改字段都就地做，
    /// 列表本身不重建（重建会把滚动位置冲回顶部）。
    function syncRows() {
        var wanted = page.backend && page.backend.tasks ? page.backend.tasks : []
        var wantedIds = {}
        for (var i = 0; i < wanted.length; i++)
            wantedIds[wanted[i].id] = true

        // 1. 倒序删掉已经不在列表里的任务
        for (var stale = taskRows.count - 1; stale >= 0; stale--) {
            if (wantedIds[taskRows.get(stale).key] === undefined)
                taskRows.remove(stale)
        }

        // 2. 缺的先追加到末尾，保证下一步行数与目标一致
        var present = {}
        for (var p = 0; p < taskRows.count; p++)
            present[taskRows.get(p).key] = true
        for (var add = 0; add < wanted.length; add++) {
            if (present[wanted[add].id] === undefined) {
                taskRows.append(normalize(wanted[add]))
                present[wanted[add].id] = true
            }
        }

        // 3. 从尾往前对齐顺序：后面的位置已经固定，要的行一定在前面
        for (var target = wanted.length - 1; target >= 0 && target < taskRows.count; target--) {
            var id = wanted[target].id
            if (taskRows.get(target).key === id)
                continue
            var from = -1
            for (var scan = target - 1; scan >= 0; scan--) {
                if (taskRows.get(scan).key === id) {
                    from = scan
                    break
                }
            }
            if (from >= 0)
                taskRows.move(from, target, 1)
        }

        // 4. 理论上不会多出行；防御性删掉，避免残留
        while (taskRows.count > wanted.length)
            taskRows.remove(taskRows.count - 1)

        // 位置对上后，再补字段差异（进度 / 状态 / 说明）
        for (var at = 0; at < wanted.length && at < taskRows.count; at++) {
            var item = normalize(wanted[at])
            var current = taskRows.get(at)
            for (var field in item) {
                if (current[field] !== item[field])
                    taskRows.setProperty(at, field, item[field])
            }
        }
    }

    /// 只留列表需要的字段（`id` 换个名字，免得和 QML 的 id 概念混在一起）。
    function normalize(task) {
        return {
            key: task.id,
            name: task.name,
            kind: task.kind,
            phase: task.phase,
            phaseText: task.phaseText,
            progress: task.progress,
            progressText: task.progressText,
            sizeText: task.sizeText,
            createdText: task.createdText,
            detail: task.detail,
            settled: task.settled,
            fileId: task.fileId || "",
            hasThumbnail: task.hasThumbnail === true,
            video: task.video === true,
            playable: task.playable === true
        }
    }

    TaskContextMenu {
        id: taskMenu
        backend: page.backend
        onDeleteRequested: function (id, name) {
            page.pendingDeleteId = id
            page.pendingDeleteName = name
            deleteDialog.open()
        }
        onOpenFolderRequested: function (fileId) {
            page.openFolderRequested(fileId)
        }
        onRevealLocationRequested: function (fileId) {
            page.revealLocationRequested(fileId)
        }
        onPlayRequested: function (fileId, point) {
            page.playRequested(fileId, point)
        }
    }

    KDialog {
        id: deleteDialog
        objectName: "deleteTaskDialog"
        title: qsTr("删除任务")
        modal: true
        anchors.centerIn: parent
        width: 380 + leftPadding + rightPadding
        footer: StandardDialogFooter {
            dialog: deleteDialog
        }

        Label {
            width: 380
            wrapMode: Text.Wrap
            text: qsTr("确定删除任务「%1」吗？\n只会移除这条任务记录，已下载的云端文件不会删除。")
                .arg(page.pendingDeleteName)
        }

        onAccepted: page.backend.deleteTask(page.pendingDeleteId)
    }

    NewTaskDialog {
        id: newTaskDialog
        backend: page.backend
    }
}
