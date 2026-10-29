import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 离线任务页：粘贴磁力/URL 回车提交，列表由后台轮询自动刷新。
// 顶层用 Item 而非 Page：Page 自带 header/footer，放进 StackLayout 容易撑爆高度。
Item {
    id: page

    property var backend: null
    /// 页面被侧栏切到前台：首次进入立即刷新，之后由 Rust 冷却与轮询合并请求。
    property bool active: false
    property string pendingDeleteId: ""
    property string pendingDeleteName: ""

    readonly property bool tasksLoading: backend ? backend.tasksLoading : false
    readonly property bool showEmptyState: taskRows.count === 0 && !tasksLoading
    readonly property bool showLoadingState: taskRows.count === 0 && tasksLoading
    readonly property int tableHeaderHeight: 34
    readonly property int tableRowHeight: 56
    readonly property int statusColumnWidth: 88
    readonly property int progressColumnWidth: 150
    readonly property int actionColumnWidth: 64
    readonly property int tableColumnSpacing: 12
    readonly property int tableContentPadding: 12

    onActiveChanged: {
        if (active && backend)
            backend.refreshTasks()
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
        anchors.fill: parent
        // 右边不留：列表铺到窗口右缘，滚动条贴边；
        // 输入行和状态行自己补右边距
        anchors.leftMargin: metrics.pageMargin
        anchors.topMargin: metrics.pageMargin
        anchors.bottomMargin: metrics.pageMargin
        spacing: theme.gapSm

        RowLayout {
            Layout.fillWidth: true
            Layout.rightMargin: metrics.pageMargin
            spacing: theme.gapSm

            KTextField {
                id: urlField
                Layout.fillWidth: true
                placeholderText: qsTr("粘贴磁力链接或下载地址，回车提交")
                selectByMouse: true
                text: page.backend ? page.backend.taskInput : ""
                onAccepted: page.submit()
                onTextEdited: if (page.backend) page.backend.taskInput = text
            }

            KButton {
                text: qsTr("提交")
                variant: "primary"
                implicitHeight: 36
                enabled: urlField.text.trim().length > 0
                onClicked: page.submit()
            }

            KButton {
                text: qsTr("刷新")
                variant: "secondary"
                implicitHeight: 36
                enabled: page.backend ? !page.tasksLoading : false
                onClicked: page.backend.refreshTasks()
            }
        }

        Label {
            visible: page.activeCount > 0
            Layout.rightMargin: metrics.pageMargin
            text: qsTr("%1 条任务进行中，列表每 10 秒自动刷新").arg(page.activeCount)
            color: theme.textMuted
            font.pixelSize: theme.fontMeta
        }

        TaskTableHeader {
            Layout.fillWidth: true
            Layout.preferredHeight: page.tableHeaderHeight
            rowHeight: page.tableHeaderHeight
            statusColumnWidth: page.statusColumnWidth
            progressColumnWidth: page.progressColumnWidth
            actionColumnWidth: page.actionColumnWidth
            columnSpacing: page.tableColumnSpacing
            contentPadding: page.tableContentPadding
        }

        ListView {
            id: list
            objectName: "tasksList"
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            spacing: 2
            model: taskRows

            // 列表铺到窗口右缘，条画在右侧 10px 车道里
            ScrollBar.vertical: KScrollBar {}

            delegate: TaskRow {
                // 行宽减去滚动条车道，右边不会压到条
                width: list.width - metrics.scrollBarSlot
                rowHeight: page.tableRowHeight
                statusColumnWidth: page.statusColumnWidth
                progressColumnWidth: page.progressColumnWidth
                actionColumnWidth: page.actionColumnWidth
                columnSpacing: page.tableColumnSpacing
                contentPadding: page.tableContentPadding
                // ⚠ 这里**不要**再写 `key: model.key ?? ""` 这类绑定：TaskRow 的字段
                // 都是 required property，ListView 会按同名角色自动注入；一旦手写
                // `model.xxx` 覆盖上去，自动注入就被顶掉，而 ListModel 的 `model`
                // 对象在这里读不到这些角色 → 整张卡片字段全空（只剩空壳和删除按钮）。
                onDeleteRequested: function (key, name) {
                    page.pendingDeleteId = key
                    page.pendingDeleteName = name
                    deleteDialog.open()
                }
            }

            Label {
                objectName: "tasksEmptyState"
                anchors.centerIn: parent
                visible: page.showEmptyState
                text: qsTr("暂无离线任务\n把磁力链接或下载地址粘到上方即可开始")
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

    function submit() {
        var url = urlField.text.trim()
        if (url.length === 0)
            return
        page.backend.addTask(url)
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
            phase: task.phase,
            phaseText: task.phaseText,
            progress: task.progress,
            progressText: task.progressText,
            sizeText: task.sizeText,
            createdText: task.createdText,
            detail: task.detail,
            settled: task.settled
        }
    }

    KDialog {
        id: deleteDialog
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
}
