import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 传输任务页：进行中、排队和历史共用一张表。
//
// 文件夹批次默认只占父行；点名称或箭头后，子文件按 index 就地插在父行下面。
// 同一时间只展开一个文件夹。进度心跳只刷新 transferLive，不重建 ListModel。
Item {
    id: page

    property var backend: null
    /// 主窗口宽度。响应式阈值按用户实际看到的窗口判断，不按内容区宽度判断。
    property int viewportWidth: 0

    /// 记录结构版本：只有增删 / 状态变化才重新对齐 ListModel。
    property int revision: -1

    /// 来源条目 id（文件页重复点已占用条目时定位到这里）。
    property string focusSourceId: ""
    property string highlightedKey: ""

    /// 当前唯一展开的文件夹父行 key。
    property string expandedKey: ""
    property int expandedIndex: -1
    property int expandedChildCount: 0
    property var expandedRowData: null

    readonly property real responsiveWidth: viewportWidth > 0 ? viewportWidth : width
    readonly property bool showRate: responsiveWidth >= 1180
    readonly property bool showSize: responsiveWidth >= 980
    readonly property int stickyTop: expandedIndex >= 0
                                       ? expandedIndex * tableDensity.rowHeight : 0
    readonly property int stickyBottom: expandedIndex >= 0
                                         ? (expandedIndex + expandedChildCount + 1)
                                           * tableDensity.rowHeight : 0
    readonly property bool stickyVisible: expandedRowData !== null
                                          && list.height > 0
                                          && list.contentY > stickyTop
                                          && list.contentY < stickyBottom

    readonly property int activeCount: backend ? backend.downloadActive : 0
    readonly property int queuedCount: backend ? backend.downloadQueued : 0
    readonly property int pausedCount: {
        var source = backend ? backend.transfers || [] : []
        var count = 0
        for (var i = 0; i < source.length; i++) {
            if (source[i].folder !== undefined && source[i].state === "paused")
                count += 1
        }
        return count
    }
    readonly property int workingCount: Math.max(activeCount - queuedCount, 0)
    readonly property bool hasFinished: backend !== null
                                         && backend.transferHasFinished
    readonly property string statusText: {
        var parts = []
        if (workingCount > 0)
            parts.push(qsTr("%1 项传输中").arg(workingCount))
        if (queuedCount > 0)
            parts.push(workingCount > 0 || pausedCount > 0
                       ? qsTr("%1 项等待").arg(queuedCount)
                       : qsTr("%1 项等待开始").arg(queuedCount))
        if (pausedCount > 0)
            parts.push(qsTr("%1 项已暂停").arg(pausedCount))
        if (parts.length > 0)
            return parts.join(" · ")
        return qsTr("暂无传输任务")
    }

    UiMetrics { id: metrics }
    KTheme { id: theme }
    TransferTableDensity { id: tableDensity }

    ListModel { id: rows }

    Connections {
        target: page.backend

        function onChanged() {
            if (!page.backend)
                return
            if (page.backend.transferRevision !== page.revision) {
                page.revision = page.backend.transferRevision
                page.syncRows()
            }
        }
    }

    Component.onCompleted: {
        page.revision = page.backend ? page.backend.transferRevision : -1
        page.syncRows()
    }

    Timer {
        id: highlightTimer
        interval: 1200
        repeat: false
        onTriggered: page.highlightedKey = ""
    }

    readonly property var liveByKey: {
        var map = {}
        var list = page.backend ? page.backend.transferLive : []
        for (var i = 0; i < list.length; i++)
            map[list[i].key] = list[i]
        return map
    }

    function isActivePhase(phase) {
        return phase === "queued" || phase === "preparing"
                || phase === "running" || phase === "pausing"
    }

    function isExpandable(folder) {
        return folder !== null && folder !== undefined
                && folder.folder === true
                && folder.childCount > 0
                && folder.state !== "preparing"
    }

    function sourceFolder(key) {
        var source = page.backend ? page.backend.transfers || [] : []
        for (var i = 0; i < source.length; i++) {
            if (source[i].key === key && page.isExpandable(source[i]))
                return source[i]
        }
        return null
    }

    function folderScreenY(key) {
        if (!list)
            return -1
        var currentIndex = -1
        for (var row = 0; row < rows.count; row++) {
            if (rows.get(row).key === key && rows.get(row).rowKind === "folder") {
                currentIndex = row
                break
            }
        }
        return currentIndex < 0 ? -1
                             : currentIndex * tableDensity.rowHeight - list.contentY
    }

    /// 切换展开项会同时删除旧子行、插入新子行。先记下被点父行的屏幕位置，
    /// 等模型改完后重挂一次同一个 ListModel 并设 contentY，避免委托保留旧 y。
    function restoreFolderPosition(key, screenY) {
        if (!list || screenY < 0)
            return
        var currentIndex = -1
        for (var row = 0; row < rows.count; row++) {
            if (rows.get(row).key === key && rows.get(row).rowKind === "folder") {
                currentIndex = row
                break
            }
        }
        if (currentIndex < 0)
            return
        var targetY = Math.max(0,
            currentIndex * tableDensity.rowHeight - screenY)
        // forceLayout 仍可能留下旧委托；同步卸载 / 重挂同一模型可强制完整重排。
        // 仅用于用户切换展开项的这一次结构变化，普通轮询同步绝不能这样重灌。
        var model = list.model
        list.model = null
        list.model = model
        var maxY = Math.max(0, list.contentHeight - list.height)
        list.contentY = Math.min(targetY, maxY)
    }

    function toggleFolder(key) {
        if (!page.backend || page.sourceFolder(key) === null)
            return
        if (page.expandedKey === key) {
            var parentIndex = page.expandedIndex
            var restoreParent = page.stickyVisible
            if (restoreParent)
                page.positionExpandedParent(parentIndex)
            page.expandedKey = ""
            page.backend.closeTransferFolder()
            page.syncRows()
            return
        }

        var switchScreenY = page.expandedKey.length > 0
                            ? page.folderScreenY(key) : -1
        page.expandedKey = key
        page.backend.openTransferFolder(key)
        page.syncRows()
        if (!page.backend.transferDetailActive)
            page.expandedKey = ""
        else if (switchScreenY >= 0)
            page.restoreFolderPosition(key, switchScreenY)
    }

    function positionExpandedParent(index) {
        if (!list || index < 0 || index >= rows.count)
            return
        var maxY = Math.max(0, list.contentHeight - list.height)
        list.contentY = Math.min(index * tableDensity.rowHeight, maxY)
    }

    function normalizeParent(item) {
        var notice = item.state === "notice"
        return {
            modelKey: "top:" + item.key,
            key: item.key,
            name: item.name,
            sourceId: item.sourceId || "",
            phase: item.state,
            stateText: item.stateText || "",
            running: item.running === true,
            progress: item.progress || 0,
            progressText: item.progressText || "",
            sizeText: item.sizeText || "",
            detail: item.detail || "",
            path: item.path || "",
            folder: item.folder === true,
            childCount: item.childCount || 0,
            failedCount: item.failedCount || 0,
            depth: 0,
            rowKind: notice ? "notice" : (item.folder === true ? "folder" : "file"),
            parentKey: "",
            expandable: !notice && page.isExpandable(item),
            expanded: !notice && page.expandedKey === item.key
        }
    }

    function normalizeChild(item) {
        var notice = item.state === "notice"
        return {
            modelKey: "child:" + item.key,
            key: item.key,
            name: item.name,
            sourceId: item.sourceId || "",
            phase: item.state,
            stateText: item.stateText || "",
            running: item.running === true,
            progress: item.progress || 0,
            progressText: item.progressText || "",
            sizeText: item.sizeText || "",
            detail: item.detail || "",
            path: item.path || "",
            folder: false,
            childCount: 0,
            failedCount: 0,
            depth: notice ? 0 : 1,
            rowKind: notice ? "notice" : "child",
            parentKey: page.expandedKey,
            expandable: false,
            expanded: false
        }
    }

    function wantedRows() {
        var source = page.backend ? page.backend.transfers || [] : []
        var expandedFolder = page.sourceFolder(page.expandedKey)
        if (page.expandedKey.length > 0
                && (expandedFolder === null || !page.backend.transferDetailActive)) {
            page.expandedKey = ""
            expandedFolder = null
        }

        var wanted = []
        for (var i = 0; i < source.length; i++) {
            var item = source[i]
            wanted.push(page.normalizeParent(item))
            if (expandedFolder === null || item.key !== page.expandedKey)
                continue
            var children = page.backend.transferDetailActive
                         ? page.backend.transferDetail || [] : []
            for (var child = 0; child < children.length; child++)
                wanted.push(page.normalizeChild(children[child]))
        }
        return wanted
    }

    /// 按 modelKey 就地增删 / 搬移 / 补字段，避免整表重灌冲回顶部。
    function syncRows() {
        var wanted = page.wantedRows()
        var wantedKeys = {}
        for (var i = 0; i < wanted.length; i++)
            wantedKeys[wanted[i].modelKey] = true

        for (var stale = rows.count - 1; stale >= 0; stale--) {
            if (wantedKeys[rows.get(stale).modelKey] === undefined)
                rows.remove(stale)
        }

        var present = {}
        for (var p = 0; p < rows.count; p++)
            present[rows.get(p).modelKey] = true
        for (var add = 0; add < wanted.length; add++) {
            if (present[wanted[add].modelKey] === undefined) {
                rows.append(wanted[add])
                present[wanted[add].modelKey] = true
            }
        }

        for (var target = wanted.length - 1; target >= 0 && target < rows.count; target--) {
            var modelKey = wanted[target].modelKey
            if (rows.get(target).modelKey === modelKey)
                continue
            var from = -1
            for (var scan = target - 1; scan >= 0; scan--) {
                if (rows.get(scan).modelKey === modelKey) {
                    from = scan
                    break
                }
            }
            if (from >= 0)
                rows.move(from, target, 1)
        }

        while (rows.count > wanted.length)
            rows.remove(rows.count - 1)

        for (var at = 0; at < wanted.length && at < rows.count; at++) {
            var next = wanted[at]
            var current = rows.get(at)
            for (var field in next) {
                if (current[field] !== next[field])
                    rows.setProperty(at, field, next[field])
            }
        }

        var stickyIndex = -1
        var stickyChildren = 0
        for (var find = 0; find < wanted.length; find++) {
            if (wanted[find].key !== page.expandedKey
                    || wanted[find].rowKind !== "folder")
                continue
            stickyIndex = find
            for (var child = find + 1;
                 child < wanted.length && wanted[child].rowKind === "child";
                 child++)
                stickyChildren += 1
            break
        }
        page.expandedIndex = stickyIndex
        page.expandedChildCount = stickyChildren
        page.expandedRowData = stickyIndex >= 0 ? wanted[stickyIndex] : null
    }

    function focusSource(sourceId) {
        if (sourceId.length === 0)
            return
        page.focusSourceId = sourceId
        Qt.callLater(page.locate)
    }

    function locate() {
        if (page.focusSourceId.length === 0)
            return
        page.syncRows()

        var fallback = -1
        for (var i = 0; i < rows.count; i++) {
            if (rows.get(i).sourceId !== page.focusSourceId)
                continue
            if (fallback < 0)
                fallback = i
            if (!page.isActivePhase(rows.get(i).phase))
                continue
            page.showRowAt(i)
            return
        }
        if (fallback >= 0)
            page.showRowAt(fallback)
    }

    function showRowAt(index) {
        list.positionViewAtIndex(index, ListView.Contain)
        page.highlightedKey = rows.get(index).key
        highlightTimer.restart()
    }

    readonly property string clearConfirmMessage: qsTr(
        "将清空所有已结束的传输记录（完成 / 失败 / 取消）。\n"
        + "进行中、排队与已中断的记录会保留。确定清空？")

    function confirmClear() {
        if (page.backend)
            clearDialog.open()
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.leftMargin: metrics.pageMargin
        anchors.topMargin: metrics.pageMargin
        anchors.bottomMargin: metrics.pageMargin
        spacing: theme.gapXs

        TransferTableHeader {
            Layout.fillWidth: true
            showRate: page.showRate
            showSize: page.showSize
        }

        Item {
            id: listHost
            Layout.fillWidth: true
            Layout.fillHeight: true

            ListView {
                id: list
                objectName: "transferList"
                anchors.fill: parent
                clip: true
                spacing: 0
                model: rows
                boundsBehavior: Flickable.StopAtBounds
                cacheBuffer: 400
                ScrollBar.vertical: KScrollBar {}

                delegate: TransferMainRow {
                    width: list.width - metrics.scrollBarSlot
                    live: page.liveByKey
                    showSize: page.showSize
                    showRate: page.showRate
                    flash: key === page.highlightedKey
                    onToggleRequested: function(rowKey) {
                        page.toggleFolder(rowKey)
                    }
                    onPauseRequested: function(rowKey) {
                        page.backend.pauseTransfer(rowKey)
                    }
                    onResumeRequested: function(rowKey) {
                        page.backend.resumeTransfer(rowKey)
                    }
                    onCancelRequested: function(rowKey) {
                        page.backend.cancelTransfer(rowKey)
                    }
                    onRetryRequested: function(rowKey) {
                        page.backend.retryTransfer(rowKey)
                    }
                    onRevealRequested: function(rowKey) {
                        page.backend.revealTransfer(rowKey)
                    }
                }

                Label {
                    objectName: "emptyHint"
                    anchors.centerIn: parent
                    visible: list.count === 0
                    text: qsTr("暂无传输任务\n在文件页右键选择下载即可开始")
                    horizontalAlignment: Text.AlignHCenter
                    color: theme.textMuted
                    font.pixelSize: theme.fontBody
                }
            }

            TransferStickyRow {
                width: list.width - metrics.scrollBarSlot
                rowData: page.expandedRowData
                live: page.liveByKey
                wheelTarget: list
                showSize: page.showSize
                showRate: page.showRate
                flash: page.expandedRowData !== null
                       && page.expandedRowData.key === page.highlightedKey
                visible: page.stickyVisible
                anchors.left: list.left
                anchors.top: list.top
                z: 2
                onToggleRequested: function(rowKey) {
                    page.toggleFolder(rowKey)
                }
                onPauseRequested: function(rowKey) {
                    page.backend.pauseTransfer(rowKey)
                }
                onResumeRequested: function(rowKey) {
                    page.backend.resumeTransfer(rowKey)
                }
                onCancelRequested: function(rowKey) {
                    page.backend.cancelTransfer(rowKey)
                }
                onRetryRequested: function(rowKey) {
                    page.backend.retryTransfer(rowKey)
                }
                onRevealRequested: function(rowKey) {
                    page.backend.revealTransfer(rowKey)
                }
            }
        }
    }

    KDialog {
        id: clearDialog
        objectName: "clearConfirmDialog"
        parent: Overlay.overlay
        title: qsTr("清空传输记录")
        modal: true
        anchors.centerIn: Overlay.overlay
        width: 380 + leftPadding + rightPadding
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

        onAccepted: {
            if (page.backend)
                page.backend.clearFinishedTransfers()
        }
    }
}
