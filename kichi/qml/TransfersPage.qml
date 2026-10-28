import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 传输任务页：下载记录（排队 / 准备中 / 下载中 / 完成 / 失败 / 取消 / 已中断）。
//
// 主列表一个下载批次一行：文件夹批次是文件夹名，单文件批次是文件名。
// 点有子文件的文件夹行进入 TransferFolderPage（独立详情页），返回时主列表
// 保持原滚动位置——所以两个页面都常驻在 StackLayout 里，绝不用 Loader 重建。
//
// 记录跨重启保留（`docs/adr/0014-download-history-on-disk.md`）：工具条下面
// 一行日期分组（今天 / 昨天 / 更早 ▾），默认只看今天；进行中 / 排队 / 已中断
// 的行不受日期过滤，永远排最上面。
//
// 数据全部来自 backend.transfers（Rust 侧已排好序并已按日期过滤），
// 本页只负责显示与操作入口。
Item {
    id: page

    property var backend: null

    /// 记录的结构版本：Rust 只在结构变化时 +1。
    /// 进度节拍只更新 transferLive（见 below），不碰列表。
    property int revision: -1

    /// 要定位到的来源条目 id（由 main.qml 在下载时写入）。
    property string focusSourceId: ""

    /// 当前高亮的行 key（约 1.2 秒后淡出）
    property string highlightedKey: ""

    /// 选中的日期分组（`today` / `YYYY-MM-DD`）；由 Rust 侧保存与过滤
    property string selectedDay: "today"

    /// 历史上出现过的日子：[{ key, label }]，新 → 旧
    property var dayGroups: []

    /// 分组行里直接列出的日子数（再早的收进「更早 ▾」）
    property int inlineDays: 4
    /// 直接列出的日子（最多 inlineDays 个）
    readonly property var inlineDayGroups: page.dayGroups.slice(0, page.inlineDays)
    /// 收进「更早 ▾」的日子
    readonly property var earlierDayGroups: page.dayGroups.slice(page.inlineDays)

    /// 文件夹详情是否打开（Rust 侧选中状态）
    readonly property bool detailActive: page.backend !== null
                                           && page.backend.transferDetailActive

    /// 当前详情页对应的文件夹行 key / 复制过来的行对象
    property string detailKey: ""
    property var detailFolder: null

    // 页面尺寸常量：滚动条车道（见 KScrollBar / docs/pitfalls.md 第 13 条）
    UiMetrics {
        id: metrics
    }

    // —— 列表模型 ——
    // ⚠ 不能用「每次拿一个新数组赋给 ListView.model」：那会让 ListView 整表
    // 重灌，滚动位置被冲回顶部；即便事后把位置放回去，中间也会闪一帧列表顶部
    // （那里正是「正在下载」那条的进度条，2026-10-27 用户实测看到闪框）。
    // 这里用 ListModel 就地做差异：移动 / 改字段 / 增删，列表不重建。
    ListModel {
        id: rows
    }

    Connections {
        target: page.backend

        function onChanged() {
            if (!page.backend)
                return
            // 记录被清空 / 淘汰 / 登出 / 子行全部被重试：Rust 已清掉选中
            if (!page.backend.transferDetailActive && page.detailKey.length > 0) {
                page.detailKey = ""
                page.detailFolder = null
            }
            if (page.backend.transferRevision !== page.revision) {
                page.revision = page.backend.transferRevision
                page.refreshDays()
                page.syncRows()
            }
        }
    }

    Component.onCompleted: {
        page.refreshDays()
        syncRows()
    }

    // —— 日期分组 ——

    /// 向 Rust 要一次「历史上有哪些日子」。
    ///
    /// 选中的分组由本页保存（`selectedDay`，默认今天）：它是显示状态，
    /// Rust 只负责按它过滤。
    function refreshDays() {
        if (!page.backend)
            return
        var days = page.backend.transferHistoryDays()
        // 复制成纯 JS 数组：QVariantList 是 array-like，直接 slice 不一定可用
        var copy = []
        for (var i = 0; i < days.length; i++)
            copy.push({ key: days[i].key, label: days[i].label })
        page.dayGroups = copy
    }

    /// 选中一个日期分组（Rust 侧按它过滤后重新发 transfers）。
    function selectDay(day) {
        if (!page.backend || day.length === 0)
            return
        page.selectedDay = day
        page.backend.setTransferHistoryDay(day)
        page.refreshDays()
    }

    /// 「更早 ▾」里的月份分隔文字（跨月时插一个，按年-月分组）。
    function monthLabel(day) {
        var parts = String(day).split("-")
        if (parts.length < 2)
            return ""
        return qsTr("%1 年 %2 月").arg(parts[0]).arg(Number(parts[1]))
    }

    /// 下一个分组是不是换月了（决定菜单里要不要插月份标题）。
    function monthChangedBefore(index) {
        if (index <= 0)
            return true
        var previous = page.earlierDayGroups[index - 1]
        var current = page.earlierDayGroups[index]
        if (!previous || !current)
            return false
        return String(previous.key).slice(0, 7) !== String(current.key).slice(0, 7)
    }

    // —— 文件夹详情：下钻与返回 ——

    function openFolder(key) {
        if (!page.backend || !key || key.length === 0)
            return
        page.detailKey = key
        page.backend.openTransferFolder(key)
        page.refreshDetailFolder()
        // 无效 key（排队中 / 空文件夹）不会激活详情，清掉避免残留
        if (!page.backend.transferDetailActive) {
            page.detailKey = ""
            page.detailFolder = null
        }
    }

    function closeFolder() {
        page.detailKey = ""
        page.detailFolder = null
        if (page.backend)
            page.backend.closeTransferFolder()
    }

    /// 从主列表复制当前文件夹行给详情页头部（每次结构变化后刷新）。
    function refreshDetailFolder() {
        if (!page.backend || page.detailKey.length === 0) {
            page.detailFolder = null
            return
        }
        for (var i = 0; i < rows.count; i++) {
            var candidate = rows.get(i)
            if (candidate.key === page.detailKey) {
                page.detailFolder = page.folderSnapshot(candidate)
                return
            }
        }
        page.detailFolder = null
    }

    function folderSnapshot(row) {
        return {
            key: row.key,
            name: row.name,
            sourceId: row.sourceId,
            phase: row.phase,
            stateText: row.stateText,
            running: row.running,
            progress: row.progress,
            progressText: row.progressText,
            sizeText: row.sizeText,
            detail: row.detail,
            path: row.path,
            folder: row.folder,
            childCount: row.childCount,
            failedCount: row.failedCount
        }
    }

    // —— 定位：点已占用的条目时跳过来看（main.qml 收到 locateTransfer 后调用）——
    // 用直接调用而非监听属性：同一条目连点两次时属性值不变、changed 不会再来；
    // 记录可能刚变，等一帧（Qt.callLater 在 syncRows 之后跑）。
    function focusSource(sourceId) {
        if (sourceId.length === 0)
            return
        // 详情页开着时先回主列表，否则定位会把用户留在详情页
        if (page.detailActive)
            page.closeFolder()
        page.focusSourceId = sourceId
        Qt.callLater(page.locate)
    }

    Timer {
        id: highlightTimer
        interval: 1200
        repeat: false
        onTriggered: page.highlightedKey = ""
    }

    /// 滚到 focusSourceId 对应的第一条进行中行并短暂高亮。
    function locate() {
        if (page.focusSourceId.length === 0)
            return
        page.syncRows()
        for (var i = 0; i < rows.count; i++) {
            var row = rows.get(i)
            if (row.sourceId !== page.focusSourceId)
                continue
            if (!page.isActivePhase(row.phase))
                continue
            list.positionViewAtIndex(i, ListView.Contain)
            page.highlightedKey = row.key
            highlightTimer.restart()
            return
        }
        // 找不到 active 行时退而求其次：已经结束的那条也给用户看一眼
        for (var done = 0; done < rows.count; done++) {
            if (rows.get(done).sourceId === page.focusSourceId) {
                list.positionViewAtIndex(done, ListView.Contain)
                page.highlightedKey = rows.get(done).key
                highlightTimer.restart()
                return
            }
        }
    }

    function isActivePhase(phase) {
        return phase === "queued" || phase === "preparing" || phase === "running"
    }

    /// 把 Rust 的快照贴到 ListModel 上（只动差异，不整表重灌）。
    ///
    /// 关键一：一次结构变化只做**最少几次** move。逐格挪会把 `ListView` 的内部
    /// 行位置搅乱，随后被强制重排、跳到别处（2026-10-27 用户实测：滚到中间，
    /// 每完成一项就往下跳一次）。一条记录完成时，它要从「进行中」段挪到
    /// 「已完成」段——正向扫描会把它一格一格往后推（一次完成 = 几十次 move），
    /// 所以这里**从尾往前**对齐：位置正确的行一次都不动，一条完成最多一次 move。
    ///
    /// 关键二：**move 的目标位置必须小于当前 count**，等于 count 会被 Qt 判
    /// `move: out of range` 并直接丢弃（不是 clamp）——游标不前进就是死循环。
    /// 从尾往前时 `target` 只会小于 count，天然不会踩到。
    ///
    /// 做法（每一步都不会越界）：
    /// 1. 倒序删掉不在快照里的行；
    /// 2. 快照里有、模型里没有的行先追加到末尾（位置交给第 3 步修正）；
    /// 3. 从尾往前逐位对：对不上就把这一行从**前面**移过来（后面的位置都已固定，
    ///    所以它必然在前面）；
    /// 4. 多出来的行删掉。
    ///
    /// 角色名不能叫 `state`（`Item` 自带同名属性，qmllint 报 property-override，
    /// 运行时也读不到角色值），因此统一映射成 `phase`。
    function syncRows() {
        if (!page.backend)
            return
        var wanted = page.backend.transfers
        var wantedKeys = {}
        for (var i = 0; i < wanted.length; i++)
            wantedKeys[wanted[i].key] = true

        // 1. 倒序删：边删边倒着走，下标才不会错位
        for (var stale = rows.count - 1; stale >= 0; stale--) {
            if (wantedKeys[rows.get(stale).key] === undefined)
                rows.remove(stale)
        }

        // 2. 缺的行先追加到末尾，保证下一步 count 与快照一致
        var present = {}
        for (var p = 0; p < rows.count; p++)
            present[rows.get(p).key] = true
        for (var add = 0; add < wanted.length; add++) {
            if (present[wanted[add].key] === undefined) {
                rows.append(normalize(wanted[add]))
                present[wanted[add].key] = true
            }
        }

        // 3. 从尾往前对齐：后面的位置已固定，故这一步要的行一定在前面
        for (var target = wanted.length - 1; target >= 0 && target < rows.count; target--) {
            var key = wanted[target].key
            if (rows.get(target).key === key)
                continue
            var from = -1
            for (var scan = target - 1; scan >= 0; scan--) {
                if (rows.get(scan).key === key) {
                    from = scan
                    break
                }
            }
            if (from >= 0)
                rows.move(from, target, 1)
        }

        // 4. 理论上不会有多出来的行；防御性删掉，避免残留
        while (rows.count > wanted.length)
            rows.remove(rows.count - 1)

        // 位置对上了，再补字段差异（状态 / 大小 / 说明）
        for (var at = 0; at < wanted.length && at < rows.count; at++) {
            var item = normalize(wanted[at])
            var current = rows.get(at)
            for (var field in item) {
                if (current[field] !== item[field])
                    rows.setProperty(at, field, item[field])
            }
        }

        if (page.detailActive)
            page.refreshDetailFolder()
    }

    function normalize(item) {
        return {
            key: item.key,
            name: item.name,
            sourceId: item.sourceId || "",
            phase: item.state,
            stateText: item.stateText,
            running: item.running,
            progress: item.progress,
            progressText: item.progressText,
            sizeText: item.sizeText,
            detail: item.detail,
            path: item.path || "",
            folder: item.folder === true,
            childCount: item.childCount || 0,
            failedCount: item.failedCount || 0
        }
    }

    /// 有没有可清的已结束记录（按钮据此置灰）。
    ///
    /// 看**全部历史**而不是当前这一天：清空清的是全部历史里的已结束批次。
    readonly property bool hasFinished: page.backend !== null
                                       && page.backend.transferHasFinished

    /// 清空确认框的正文（测试也读这一份，文案只有一处）
    readonly property string clearConfirmMessage: qsTr(
        "将清空全部历史里所有已结束的记录（完成 / 失败 / 已取消），"
        + "不限于当前选中的日期。\n"
        + "进行中、排队与已中断的记录会保留。确定清空？")

    /// 二次确认「确定」之后：清全部历史里所有已结束的整批（不按日期收窄）。
    function confirmClear() {
        if (page.backend)
            page.backend.clearFinishedTransfers()
    }

    /// 进行中条数（排队 + 准备中 + 下载中；一个文件夹算 1）
    readonly property int activeCount: page.backend ? page.backend.downloadActive : 0

    /// 实时进度按 key 索引：主列表的文件夹父行与详情页的子行都从这里取。
    /// 结构变化时它也会并进 transfers / transferDetail 模型。
    readonly property var liveByKey: page.liveMapOf(
        page.backend ? page.backend.transferLive : [])

    function liveMapOf(list) {
        var map = {}
        for (var i = 0; i < list.length; i++)
            map[list[i].key] = list[i]
        return map
    }

    ColumnLayout {
        anchors.fill: parent
        // 右边不留：列表要铺到窗口右缘，让滚动条贴边；
        // 工具条 / 分隔线自己补右边距，列表内容宽度减去车道
        anchors.leftMargin: metrics.pageMargin
        anchors.topMargin: metrics.pageMargin
        anchors.bottomMargin: metrics.pageMargin
        spacing: 8

        // —— 工具条：进行中条数 + 三个操作 ——
        RowLayout {
            Layout.fillWidth: true
            // 工具条不吃滚动条车道，右缘留普通页边距（视觉位置与以前一致）
            Layout.rightMargin: metrics.pageMargin
            spacing: 8

            Label {
                Layout.fillWidth: true
                text: {
                    if (page.activeCount === 0)
                        return qsTr("没有进行中的传输")
                    var queued = page.backend ? page.backend.downloadQueued : 0
                    return queued > 0
                        ? qsTr("%1 个进行中 · %2 批排队").arg(page.activeCount).arg(queued)
                        : qsTr("%1 个进行中").arg(page.activeCount)
                }
                color: page.activeCount > 0 ? palette.text : palette.placeholderText
                font.pixelSize: 12
            }

            Button {
                text: qsTr("打开下载目录")
                onClicked: if (page.backend) page.backend.openDownloadDir()
            }

            Button {
                text: qsTr("取消下载")
                enabled: page.activeCount > 0
                onClicked: if (page.backend) page.backend.cancelDownloads()
            }

            Button {
                objectName: "clearFinishedButton"
                text: qsTr("清空已完成记录")
                enabled: page.backend !== null && page.hasFinished
                onClicked: clearDialog.open()
            }
        }

        // —— 日期分组行：今天 · 昨天 · 10-26 · 10-25 · 更早 ▾ ——
        //
        // 「今天 / 昨天」由 Rust 侧的日期文案给出；更早的日子只列月-日，
        // 超出本行能放下的部分收进「更早 ▾」（跨月时菜单里插月份标题）。
        RowLayout {
            objectName: "historyDayRow"
            Layout.fillWidth: true
            Layout.rightMargin: metrics.pageMargin
            spacing: 12
            visible: page.dayGroups.length > 0

            Repeater {
                model: page.inlineDayGroups

                delegate: Label {
                    required property var modelData
                    objectName: "dayPill-" + modelData.key
                    property string dayKey: modelData.key
                    text: modelData.label
                    font.pixelSize: 12
                    verticalAlignment: Text.AlignVCenter
                    color: page.selectedDay === dayKey ? palette.text : palette.link
                    font.bold: page.selectedDay === dayKey
                    font.underline: pillMouse.containsMouse

                    MouseArea {
                        id: pillMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: page.selectDay(dayKey)
                    }
                }
            }

            Label {
                objectName: "earlierDaysLink"
                visible: page.earlierDayGroups.length > 0
                text: qsTr("更早 ▾")
                font.pixelSize: 12
                verticalAlignment: Text.AlignVCenter
                color: palette.link
                font.bold: page.earlierDayGroups.some(function (group) {
                    return group.key === page.selectedDay
                })
                font.underline: earlierMouse.containsMouse

                MouseArea {
                    id: earlierMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    // 菜单挂在 overlay 上，坐标先换算成窗口坐标再赋值（踩坑第 10 / 11 条）
                    onClicked: {
                        var point = earlierMouse.mapToItem(null, 0, earlierMouse.height)
                        moreDaysMenu.x = point.x
                        moreDaysMenu.y = point.y
                        moreDaysMenu.open()
                    }
                }

                Menu {
                    id: moreDaysMenu
                    // 菜单一律挂在 overlay 上，坐标才与窗口一致（踩坑第 10 条）
                    parent: Overlay.overlay
                    width: 160
                    closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside

                    Repeater {
                        model: page.earlierDayGroups

                        delegate: MenuItem {
                            required property int index
                            required property var modelData
                            text: page.monthChangedBefore(index)
                                  ? page.monthLabel(modelData.key) + " · " + modelData.label
                                  : modelData.label
                            checkable: true
                            checked: page.selectedDay === modelData.key
                            onTriggered: page.selectDay(modelData.key)
                        }
                    }
                }
            }

            Item {
                Layout.fillWidth: true
            }

            Label {
                objectName: "historyHint"
                visible: page.dayGroups.length > 1
                text: qsTr("记录按天分组，进行中与已中断的始终显示")
                font.pixelSize: 12
                verticalAlignment: Text.AlignVCenter
                color: palette.placeholderText
            }
        }

        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: 1
            Layout.rightMargin: metrics.pageMargin
            color: Qt.rgba(palette.windowText.r, palette.windowText.g, palette.windowText.b, 0.10)
        }

        // 主列表与详情页都常驻：返回时主列表 contentY 不受影响。
        StackLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            currentIndex: page.detailActive ? 1 : 0

            Item {
                ListView {
                    id: list
                    objectName: "transferList"
                    anchors.fill: parent
                    clip: true
                    spacing: 2
                    model: rows
                    boundsBehavior: Flickable.StopAtBounds
                    cacheBuffer: 400

                    // 列表铺到窗口右缘，条画在右侧 10px 车道里
                    ScrollBar.vertical: KScrollBar {}

                    delegate: TransferRow {
                        id: row
                        width: list.width - metrics.scrollBarSlot
                        flash: row.key === page.highlightedKey
                        live: page.liveByKey
                        backend: page.backend
                        onOpenDetailsRequested: function(key) {
                            page.openFolder(key)
                        }
                    }

                    Label {
                        objectName: "emptyHint"
                        anchors.centerIn: parent
                        visible: list.count === 0
                        text: page.dayGroups.length === 0
                              ? qsTr("暂无传输任务\n在文件页右键选择下载即可开始")
                              : qsTr("这一天没有下载记录\n可以点上面的日期看更早的记录")
                        horizontalAlignment: Text.AlignHCenter
                        color: palette.placeholderText
                    }
                }
            }

            TransferFolderPage {
                backend: page.backend
                folderKey: page.detailKey
                folder: page.detailFolder
                live: page.liveByKey
                active: page.detailActive
                normalizeRow: page.normalize
                onBackRequested: page.closeFolder()
            }
        }
    }

    // 清空已完成记录：二次确认（清全部历史，不按日期收窄）。
    // 挂在页面根节点而不是 ColumnLayout 里：Popup 不是 Item，放进布局会用 anchors
    // 被判「被布局管理的项不能用 anchors」。
    Dialog {
        id: clearDialog
        objectName: "clearConfirmDialog"
        parent: Overlay.overlay
        title: qsTr("清空已完成记录")
        modal: true
        anchors.centerIn: Overlay.overlay
        standardButtons: Dialog.Ok | Dialog.Cancel

        Label {
            objectName: "clearConfirmText"
            text: page.clearConfirmMessage
            wrapMode: Text.Wrap
            width: 380
        }

        onAccepted: page.confirmClear()
    }
}
