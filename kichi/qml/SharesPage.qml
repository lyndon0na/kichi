import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 分享管理页：服务端列表按表格展示，支持行内复制、Ctrl/Shift/Ctrl+A 多选和批量取消分享。
Item {
    id: page

    property var backend: null
    property bool active: false
    property string copyNotice: ""
    property real savedContentY: 0
    property bool pendingScrollRestore: false
    property int scrollRestoreAttempts: 0

    readonly property bool loading: backend ? backend.sharesLoading : false
    readonly property bool actionRunning: backend ? backend.shareActionRunning : false
    readonly property bool backendSessionActive: page.backend === null
                                                 || typeof page.backend.authState === "undefined"
                                                 || page.backend.authState === "signedIn"
    readonly property bool showLoadingSkeleton: loadingSkeletonGate.showSkeleton
    readonly property int selectedCount: selection.count
    readonly property int shareCount: shareRows.count
    readonly property bool showEmptyState: shareRows.count === 0 && !loading
    readonly property bool showLoadingState: shareRows.count === 0 && loading
                                             && !showLoadingSkeleton

    readonly property int contentPadding: 12
    readonly property int columnSpacing: 12
    readonly property int iconColumnWidth: 22
    readonly property int statusColumnWidth: 100
    readonly property int expirationColumnWidth: 64
    readonly property int createdColumnWidth: 120
    readonly property int actionColumnWidth: 96
    readonly property int headerRowHeight: 34
    readonly property int rowHeight: 48

    onActiveChanged: {
        if (!active) {
            if (list)
                page.savedContentY = list.contentY
            page.pendingScrollRestore = false
            page.scrollRestoreAttempts = 0
            shareRows.clear()
            if (list) {
                list.positionViewAtBeginning()
                list.contentY = 0
            }
            return
        }
        if (list)
            list.positionViewAtBeginning()
        page.syncRows()
        page.pendingScrollRestore = true
        page.scrollRestoreAttempts = 0
        Qt.callLater(page.restoreScroll)
        if (page.backend && typeof page.backend.ensureSharesFresh === "function")
            page.backend.ensureSharesFresh()
        else
            page.refresh()
        Qt.callLater(function () { list.forceActiveFocus() })
    }

    UiMetrics { id: metrics }
    KTheme { id: theme }
    ClipboardHelper { id: clipboard }
    FileSelection { id: selection }

    LoadingSkeletonGate {
        id: loadingSkeletonGate

        active: page.active
        sessionActive: page.backendSessionActive
        loading: page.loading
        count: shareRows.count
    }

    ShareContextMenu {
        id: shareMenuView
        backend: page.backend
        onCopyRequested: function (ids) {
            page.copyShares(ids)
        }
        onRevokeRequested: function (ids) {
            page.openRevoke(ids)
        }
    }

    ShareRevokeDialog {
        id: revokeDialogView
        backend: page.backend
        onRevoked: page.onRevoked()
    }

    property alias shareMenu: shareMenuView
    property alias revokeDialog: revokeDialogView

    Timer {
        id: copyNoticeTimer
        interval: 1800
        onTriggered: page.copyNotice = ""
    }

    ListModel { id: shareRows }

    Connections {
        target: page.backend

        function onChanged() {
            if (!page.active)
                return
            page.syncRows()
        }
    }

    Component.onCompleted: syncRows()

    function restoreScroll() {
        if (!page.pendingScrollRestore || !list || list.contentHeight <= 0)
            return
        var maxY = Math.max(0, list.contentHeight - list.height)
        var target = Math.max(0, Math.min(page.savedContentY, maxY))
        list.contentY = target
        if (target >= page.savedContentY || page.scrollRestoreAttempts >= 4) {
            page.pendingScrollRestore = false
            page.scrollRestoreAttempts = 0
            return
        }
        page.scrollRestoreAttempts += 1
        Qt.callLater(page.restoreScroll)
    }

    ColumnLayout {
        objectName: "sharesTableLayout"
        anchors.fill: parent
        // 内容铺满页面，左右留白由表头 / 行内边距提供；条悬浮在右侧留白里
        anchors.topMargin: metrics.pageMargin
        anchors.bottomMargin: metrics.pageMargin
        spacing: theme.gapXs

        ShareTableHeader {
            Layout.fillWidth: true
            Layout.preferredHeight: page.headerRowHeight
            rowHeight: page.headerRowHeight
            iconColumnWidth: page.iconColumnWidth
            statusColumnWidth: page.statusColumnWidth
            expirationColumnWidth: page.expirationColumnWidth
            createdColumnWidth: page.createdColumnWidth
            actionColumnWidth: page.actionColumnWidth
            columnSpacing: page.columnSpacing
            contentPadding: page.contentPadding
        }

        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true

            ListView {
                id: list
                objectName: "sharesList"
                anchors.fill: parent
                clip: true
                spacing: 0
                model: shareRows
                boundsBehavior: Flickable.StopAtBounds
                onContentHeightChanged: page.restoreScroll()
                ScrollBar.vertical: KScrollBar {}
                focus: page.active
                Keys.onPressed: function (event) {
                    page.handleKey(event)
                }

                delegate: ShareRow {
                    id: row
                    width: list.width
                    highlighted: selection.isSelected(key)
                    rowHeight: page.rowHeight
                    iconColumnWidth: page.iconColumnWidth
                    statusColumnWidth: page.statusColumnWidth
                    expirationColumnWidth: page.expirationColumnWidth
                    createdColumnWidth: page.createdColumnWidth
                    actionColumnWidth: page.actionColumnWidth
                    columnSpacing: page.columnSpacing
                    contentPadding: page.contentPadding

                    onSelectRequested: function (modifiers, index) {
                        page.selectRow(key, index, modifiers)
                    }
                    onContextMenu: function (point) {
                        page.openRowMenu(key, row.mapToItem(null, point.x, point.y))
                    }
                    onCopyRequested: function (text, hasPassword) {
                        if (!clipboard.copyText(text))
                            return
                        page.copyNotice = hasPassword
                                ? qsTr("链接和密码已复制")
                                : qsTr("链接已复制")
                        copyNoticeTimer.restart()
                    }
                }

                Label {
                    objectName: "sharesEmptyState"
                    anchors.centerIn: parent
                    visible: page.showEmptyState
                    text: qsTr("暂无分享链接\n在文件页选中条目后右键即可创建")
                    horizontalAlignment: Text.AlignHCenter
                    color: theme.textMuted
                    font.pixelSize: theme.fontBody
                }

                Column {
                    objectName: "sharesLoadingState"
                    anchors.centerIn: parent
                    width: 220
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
                        text: qsTr("正在加载分享链接…")
                        horizontalAlignment: Text.AlignHCenter
                        color: theme.textMuted
                        font.pixelSize: theme.fontBody
                    }
                }
            }

            LoadingSkeleton {
                anchors.fill: parent
                visible: page.showLoadingSkeleton
                leadingWidth: page.iconColumnWidth
                leadingHeight: 16
                trailingColumnWidths: [page.statusColumnWidth,
                                       page.expirationColumnWidth,
                                       page.createdColumnWidth,
                                       page.actionColumnWidth]
            }

            ScrollFirstClickArea {
                id: firstClickArea
                view: list
                scrollBarSlot: metrics.scrollBarSlot

                onClickedAt: function (index, modifiers, position) {
                    const item = list.itemAtIndex(index)
                    if (!item)
                        return
                    list.forceActiveFocus()
                    firstClickArea.invokeFirstClick(
                        item, index, modifiers, position.x, position.y)
                }
            }
        }
    }

    function syncRows() {
        if (!page.active) {
            shareRows.clear()
            return
        }
        const wanted = page.backend && page.backend.shares ? page.backend.shares : []
        const wantedIds = ({})
        for (let i = 0; i < wanted.length; i++)
            wantedIds[wanted[i].id] = true

        for (let stale = shareRows.count - 1; stale >= 0; stale--) {
            if (wantedIds[shareRows.get(stale).key] === undefined)
                shareRows.remove(stale)
        }

        const present = ({})
        for (let p = 0; p < shareRows.count; p++)
            present[shareRows.get(p).key] = true
        for (let add = 0; add < wanted.length; add++) {
            if (present[wanted[add].id] === undefined) {
                shareRows.append(normalize(wanted[add]))
                present[wanted[add].id] = true
            }
        }

        for (let target = 0; target < wanted.length; target++) {
            const id = wanted[target].id
            if (shareRows.get(target).key === id)
                continue
            let from = -1
            for (let scan = target + 1; scan < shareRows.count; scan++) {
                if (shareRows.get(scan).key === id) {
                    from = scan
                    break
                }
            }
            if (from >= 0)
                shareRows.move(from, target, 1)
        }

        while (shareRows.count > wanted.length)
            shareRows.remove(shareRows.count - 1)

        for (let at = 0; at < wanted.length && at < shareRows.count; at++) {
            const item = normalize(wanted[at])
            const current = shareRows.get(at)
            for (let field in item) {
                if (current[field] !== item[field])
                    shareRows.setProperty(at, field, item[field])
            }
        }
        selection.prune(wanted)
    }

    function shareForId(id) {
        const shares = page.backend && page.backend.shares ? page.backend.shares : []
        for (let i = 0; i < shares.length; i++) {
            if (shares[i].id === id)
                return shares[i]
        }
        return null
    }

    function selectRow(id, index, modifiers) {
        const shares = page.backend && page.backend.shares ? page.backend.shares : []
        selection.click(id, index, modifiers, shares)
    }

    function selectedIdsInOrder() {
        const result = []
        const shares = page.backend && page.backend.shares ? page.backend.shares : []
        for (let i = 0; i < shares.length; i++) {
            if (selection.isSelected(shares[i].id))
                result.push(shares[i].id)
        }
        return result
    }

    function copyShares(ids) {
        const wanted = ({})
        for (let i = 0; i < (ids ? ids.length : 0); i++)
            wanted[ids[i]] = true

        const parts = []
        let hasPassword = false
        const shares = page.backend && page.backend.shares ? page.backend.shares : []
        for (let i = 0; i < shares.length; i++) {
            const share = shares[i]
            if (!wanted[share.id])
                continue
            const text = clipboard.shareText(share.url, share.passCode, share.title)
            if (text.length === 0)
                continue
            parts.push(text)
            if (String(share.passCode || "").trim().length > 0)
                hasPassword = true
        }
        if (parts.length === 0 || !clipboard.copyText(parts.join("\n\n")))
            return

        if (parts.length > 1) {
            page.copyNotice = qsTr("已复制 %1 个分享链接").arg(parts.length)
        } else {
            page.copyNotice = hasPassword
                    ? qsTr("链接和密码已复制")
                    : qsTr("链接已复制")
        }
        copyNoticeTimer.restart()
    }

    function openRowMenu(id, point) {
        const share = shareForId(id)
        if (!share)
            return
        selection.ensureSelected(share)
        shareMenu.openFor(selectedIdsInOrder(), point)
    }

    function openRevoke(ids) {
        if (!page.backend || !ids || ids.length === 0 || page.actionRunning)
            return
        const share = ids.length === 1 ? shareForId(ids[0]) : null
        const title = share ? share.title : ""
        revokeDialog.openFor(ids, title)
    }

    function onRevoked() {
        const count = revokeDialog.shareCount
        selection.clear()
        page.copyNotice = count > 1
                ? qsTr("已取消 %1 个分享").arg(count)
                : qsTr("分享已取消")
        copyNoticeTimer.restart()
    }

    function refresh() {
        if (page.backend && !page.loading)
            page.backend.refreshShares()
    }

    function handleKey(event) {
        if (!page.backend)
            return
        if (event.key === Qt.Key_A && (event.modifiers & Qt.ControlModifier)) {
            selection.selectAll(page.backend.shares ? page.backend.shares : [])
            event.accepted = true
        }
    }

    function normalize(share) {
        return {
            key: share.id,
            title: share.title,
            kind: share.kind,
            url: share.url,
            copyText: clipboard.shareText(share.url, share.passCode, share.title),
            copyHasPassword: String(share.passCode || "").trim().length > 0,
            passCode: share.passCode,
            hasPassword: share.hasPassword,
            accessText: share.accessText,
            expirationText: share.expirationText,
            createdText: share.createdText,
            statsText: share.statsText,
            statusText: share.statusText,
            statusTone: share.statusTone
        }
    }
}
