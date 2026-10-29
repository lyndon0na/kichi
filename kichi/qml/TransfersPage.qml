import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 传输任务页：顶部一张进行中摘要，下面依次是等待队列与历史。
//
// 独立文件共享「正在下载 N 个文件」摘要，不伪造文件夹标题；真实文件夹批次
// 用文件夹名作摘要标题。日期筛选与自动「今天 / 昨天」分组已移除，历史固定
// 新 → 旧。文件夹详情仍走 TransferFolderPage，两个页面常驻 StackLayout，
// 返回时主列表保持原滚动位置。
Item {
    id: page

    property var backend: null

    /// 记录的结构版本：Rust 只在结构变化时 +1。
    /// 进度节拍只更新 transferLive / transferActiveGroup，不碰列表。
    property int revision: -1

    /// 要定位到的来源条目 id（由 main.qml 在下载时写入）。
    property string focusSourceId: ""

    /// 当前高亮的行 / 摘要 lane key（约 1.2 秒后淡出）
    property string highlightedKey: ""

    /// 队列与历史条数（只统计真正显示的行，不含提示行）
    property int queueCount: 0
    property int historyCount: 0

    /// 文件夹详情是否打开（Rust 侧选中状态）
    readonly property bool detailActive: page.backend !== null
                                           && page.backend.transferDetailActive

    /// 当前详情页对应的文件夹行 key / 复制过来的行对象
    property string detailKey: ""
    property var detailFolder: null

    /// 0 或 1 个进行中分组；QML 只消费 Rust 映射，不在界面里猜批次结构。
    readonly property var activeGroup: {
        if (!page.backend)
            return null
        var groups = page.backend.transferActiveGroup
        return groups && groups.length > 0 ? groups[0] : null
    }
    readonly property bool hasActiveGroup: page.activeGroup !== null

    UiMetrics {
        id: metrics
    }

    KTheme { id: theme }

    // —— 列表模型：只放排队 / 历史，进行中在顶部摘要里 ——
    //
    // ⚠ 不能用「每次拿一个新数组赋给 ListView.model」：那会整表重灌并把
    // 滚动位置冲回顶部。这里继续按 key 就地 move / set / remove。
    ListModel {
        id: rows
    }

    Connections {
        target: page.backend

        function onChanged() {
            if (!page.backend)
                return
            // 记录被清空 / 淘汰 / 子行全部被重试：Rust 已清掉选中
            if (!page.backend.transferDetailActive && page.detailKey.length > 0) {
                page.detailKey = ""
                page.detailFolder = null
            }
            if (page.backend.transferRevision !== page.revision) {
                page.revision = page.backend.transferRevision
                page.syncRows()
            }
        }
    }

    Component.onCompleted: syncRows()

    // —— 文件夹详情：下钻与返回 ——

    function openFolder(key) {
        if (!page.backend || !key || key.length === 0)
            return
        page.detailKey = key
        page.backend.openTransferFolder(key)
        page.refreshDetailFolder()
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

    /// 从 Rust 的完整记录里复制当前文件夹总账行。进行中的文件夹不在 rows 里，
    /// 所以不能从当前可见模型反查。
    function refreshDetailFolder() {
        if (!page.backend || page.detailKey.length === 0) {
            page.detailFolder = null
            return
        }
        var source = page.backend.transfers || []
        for (var i = 0; i < source.length; i++) {
            var candidate = source[i]
            if (candidate.key === page.detailKey && candidate.folder === true) {
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
            phase: row.state,
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

    // —— 定位：点已占用的条目时跳过来看 ——

    Timer {
        id: highlightTimer
        interval: 1200
        repeat: false
        onTriggered: page.highlightedKey = ""
    }

    function focusSource(sourceId) {
        if (sourceId.length === 0)
            return
        if (page.detailActive)
            page.closeFolder()
        page.focusSourceId = sourceId
        Qt.callLater(page.locate)
    }

    function locate() {
        if (page.focusSourceId.length === 0)
            return

        // 进行中条目已经离开 rows，直接高亮摘要里的 lane / 分组。
        var group = page.activeGroup
        if (group !== null) {
            if (group.sourceId === page.focusSourceId) {
                page.highlightedKey = group.groupKey
                highlightTimer.restart()
                return
            }
            for (var lane = 0; lane < group.lanes.length; lane++) {
                if (group.lanes[lane].sourceId === page.focusSourceId) {
                    page.highlightedKey = group.lanes[lane].key
                    highlightTimer.restart()
                    return
                }
            }
        }

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

    /// 把 Rust 快照贴到 ListModel 上，只动差异，不整表重灌。
    function syncRows() {
        if (!page.backend)
            return

        var source = page.backend.transfers || []
        var queue = []
        var history = []
        var historyItems = 0
        for (var i = 0; i < source.length; i++) {
            var item = source[i]
            // 进行中在顶部摘要；排队中留在等待队列。
            if (item.state === "running" || item.state === "preparing")
                continue
            var normalized = page.normalize(item)
            normalized.sectionKey = item.state === "queued" ? "queue" : "history"
            if (normalized.sectionKey === "queue") {
                queue.push(normalized)
            } else {
                history.push(normalized)
                if (item.state !== "notice")
                    historyItems++
            }
        }
        var wanted = queue.concat(history)
        page.queueCount = queue.length
        page.historyCount = historyItems

        var wantedKeys = {}
        for (var w = 0; w < wanted.length; w++)
            wantedKeys[wanted[w].key] = true

        // 1. 倒序删掉不在快照里的行。
        for (var stale = rows.count - 1; stale >= 0; stale--) {
            if (wantedKeys[rows.get(stale).key] === undefined)
                rows.remove(stale)
        }

        // 2. 缺的行先追加到末尾。
        var present = {}
        for (var p = 0; p < rows.count; p++)
            present[rows.get(p).key] = true
        for (var add = 0; add < wanted.length; add++) {
            if (present[wanted[add].key] === undefined) {
                rows.append(wanted[add])
                present[wanted[add].key] = true
            }
        }

        // 3. 从尾往前对齐，完成一行最多 move 一次。
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

        while (rows.count > wanted.length)
            rows.remove(rows.count - 1)

        // 4. 位置对齐后再补字段差异。
        for (var at = 0; at < wanted.length && at < rows.count; at++) {
            var next = wanted[at]
            var current = rows.get(at)
            for (var field in next) {
                if (current[field] !== next[field])
                    rows.setProperty(at, field, next[field])
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
            failedCount: item.failedCount || 0,
            sectionKey: "history"
        }
    }

    readonly property bool hasFinished: page.backend !== null
                                       && page.backend.transferHasFinished

    readonly property string clearConfirmMessage: qsTr(
        "将清空所有已结束的记录（完成 / 失败 / 已取消）。\n"
        + "进行中、排队与已中断的记录会保留。确定清空？")

    function confirmClear() {
        if (page.backend)
            page.backend.clearFinishedTransfers()
    }

    readonly property int activeCount: page.backend ? page.backend.downloadActive : 0
    readonly property int queuedCount: page.backend ? page.backend.downloadQueued : 0

    readonly property string statusText: {
        if (page.activeGroup !== null) {
            if (page.activeGroup.state === "preparing")
                return qsTr("正在准备文件夹")
            if (page.activeGroup.kind === "folder") {
                return qsTr("%1 个文件正在下载 · %2 项等待")
                    .arg(page.activeGroup.runningCount).arg(page.queuedCount)
            }
            return qsTr("%1 个文件正在下载 · %2 项等待")
                .arg(page.activeGroup.runningCount).arg(page.queuedCount)
        }
        if (page.queuedCount > 0)
            return qsTr("%1 项等待开始").arg(page.queuedCount)
        return qsTr("没有进行中的传输")
    }

    /// 实时进度按 key 索引（文件夹详情页用）。
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
        anchors.leftMargin: metrics.pageMargin
        anchors.topMargin: metrics.pageMargin
        anchors.bottomMargin: metrics.pageMargin
        spacing: theme.gapMd

        RowLayout {
            Layout.fillWidth: true
            Layout.rightMargin: metrics.pageMargin
            spacing: theme.gapSm

            Label {
                Layout.fillWidth: true
                text: page.statusText
                color: page.activeGroup !== null || page.queuedCount > 0
                       ? theme.textSecondary : theme.textMuted
                font.pixelSize: theme.fontMeta
                elide: Text.ElideRight
            }

            KButton {
                text: qsTr("打开下载目录")
                variant: "secondary"
                implicitHeight: 32
                onClicked: if (page.backend) page.backend.openDownloadDir()
            }

            KButton {
                text: qsTr("取消全部")
                variant: "secondary"
                implicitHeight: 32
                enabled: page.activeCount > 0
                onClicked: if (page.backend) page.backend.cancelDownloads()
            }
        }

        TransferActiveSummary {
            objectName: "transferActiveSummary"
            Layout.fillWidth: true
            Layout.rightMargin: metrics.scrollBarSlot
            group: page.activeGroup
            highlightedKey: page.highlightedKey
            onOpenDetailsRequested: function(folderKey) {
                page.openFolder(folderKey)
            }
        }

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
                    spacing: 8
                    model: rows
                    boundsBehavior: Flickable.StopAtBounds
                    cacheBuffer: 400

                    section.property: "sectionKey"
                    section.delegate: Item {
                        objectName: section === "queue"
                                   ? "queueSectionHeader" : "historySectionHeader"
                        width: list.width - metrics.scrollBarSlot
                        height: 28

                        RowLayout {
                            anchors.left: parent.left
                            anchors.right: parent.right
                            anchors.bottom: parent.bottom
                            anchors.bottomMargin: 2
                            spacing: theme.gapSm

                            Label {
                                text: section === "queue" ? qsTr("等待队列") : qsTr("历史")
                                font.pixelSize: theme.fontSection
                                font.weight: Font.DemiBold
                                color: theme.textPrimary
                            }

                            Label {
                                objectName: section === "queue"
                                            ? "queueSectionMeta" : "historySectionMeta"
                                Layout.fillWidth: true
                                text: section === "queue"
                                    ? qsTr("%1 个批次 · 按加入顺序开始").arg(page.queueCount)
                                    : qsTr("最近记录在前 · %1 项").arg(page.historyCount)
                                font.pixelSize: theme.fontCaption
                                color: theme.textMuted
                                elide: Text.ElideRight
                            }

                            KButton {
                                objectName: "clearFinishedButton"
                                visible: section === "history"
                                         && page.backend !== null && page.hasFinished
                                text: qsTr("清空")
                                variant: "ghost"
                                implicitHeight: 24
                                leftPadding: 8
                                rightPadding: 8
                                topPadding: 2
                                bottomPadding: 2
                                font.pixelSize: theme.fontMeta
                                Layout.alignment: Qt.AlignVCenter
                                enabled: page.backend !== null && page.hasFinished
                                ToolTip.text: qsTr("清空可清理的历史记录")
                                ToolTip.visible: hovered
                                ToolTip.delay: 400
                                onClicked: clearDialog.open()
                            }
                        }
                    }

                    ScrollBar.vertical: KScrollBar {}

                    delegate: TransferMainRow {
                        width: list.width - metrics.scrollBarSlot
                        flash: key === page.highlightedKey
                        backend: page.backend
                        onOpenDetailsRequested: function(folderKey) {
                            page.openFolder(folderKey)
                        }
                    }

                    Label {
                        objectName: "emptyHint"
                        anchors.centerIn: parent
                        visible: list.count === 0 && !page.hasActiveGroup
                        text: qsTr("暂无传输任务\n在文件页右键选择下载即可开始")
                        horizontalAlignment: Text.AlignHCenter
                        color: theme.textMuted
                        font.pixelSize: theme.fontBody
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

    KDialog {
        id: clearDialog
        objectName: "clearConfirmDialog"
        parent: Overlay.overlay
        title: qsTr("清空历史")
        modal: true
        anchors.centerIn: Overlay.overlay
        footer: StandardDialogFooter {
            objectName: "clearConfirmBox"
            dialog: clearDialog
        }

        Label {
            objectName: "clearConfirmText"
            text: page.clearConfirmMessage
            wrapMode: Text.Wrap
            width: 380
        }

        onAccepted: page.confirmClear()
    }
}
