import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 分享管理页：服务端列表、复制链接 / 密码、批量撤销。
Item {
    id: page

    property var backend: null
    property bool active: false
    property var selectedMap: ({})
    property var pendingIds: []
    property string copyNotice: ""

    readonly property bool loading: backend ? backend.sharesLoading : false
    readonly property bool actionRunning: backend ? backend.shareActionRunning : false
    readonly property int selectedCount: Object.keys(selectedMap).length
    readonly property bool allSelected: shareRows.count > 0 && selectedCount === shareRows.count
    readonly property bool showEmptyState: shareRows.count === 0 && !loading
    readonly property bool showLoadingState: shareRows.count === 0 && loading

    onActiveChanged: {
        if (active && backend)
            backend.refreshShares()
    }

    UiMetrics { id: metrics }
    KTheme { id: theme }
    ClipboardHelper { id: clipboard }

    Timer {
        id: copyNoticeTimer
        interval: 1800
        onTriggered: page.copyNotice = ""
    }

    ListModel { id: shareRows }

    Connections {
        target: page.backend

        function onChanged() {
            page.syncRows()
        }

        function onShareActionDone(action, ok, message) {
            if (action !== "revoke" || !revokeDialog.visible)
                return
            if (ok) {
                page.clearSelection()
                revokeDialog.close()
            } else {
                revokeDialog.errorText = message
            }
        }
    }

    Component.onCompleted: syncRows()

    ColumnLayout {
        anchors.fill: parent
        anchors.leftMargin: metrics.pageMargin
        anchors.topMargin: metrics.pageMargin
        anchors.bottomMargin: metrics.pageMargin
        spacing: theme.gapSm

        RowLayout {
            Layout.fillWidth: true
            Layout.rightMargin: metrics.pageMargin
            spacing: theme.gapSm

            Label {
                Layout.fillWidth: true
                text: {
                    if (page.copyNotice.length > 0)
                        return page.copyNotice
                    if (page.selectedCount > 0)
                        return qsTr("已选择 %1 个分享").arg(page.selectedCount)
                    return shareRows.count > 0
                        ? qsTr("共 %1 个分享链接").arg(shareRows.count) : ""
                }
                color: page.copyNotice.length > 0 ? theme.success : theme.textMuted
                font.pixelSize: theme.fontMeta
                elide: Text.ElideRight
            }

            KButton {
                text: page.allSelected ? qsTr("取消全选") : qsTr("全选")
                variant: "ghost"
                enabled: shareRows.count > 0 && !page.actionRunning
                onClicked: page.toggleSelectAll()
            }

            KButton {
                objectName: "revokeSelectedShares"
                text: qsTr("撤销选中（%1）").arg(page.selectedCount)
                variant: "dangerOutline"
                enabled: page.selectedCount > 0 && !page.actionRunning
                onClicked: page.openRevoke(page.selectedIds())
            }

            KButton {
                text: qsTr("刷新")
                variant: "ghost"
                enabled: !page.loading
                onClicked: page.backend.refreshShares()
            }
        }

        ListView {
            id: list
            objectName: "sharesList"
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            spacing: 8
            model: shareRows
            ScrollBar.vertical: KScrollBar {}

            delegate: ShareRow {
                width: list.width - metrics.scrollBarSlot
                selected: page.isSelected(key)
                actionRunning: page.actionRunning
                onSelectionToggled: function (checked) {
                    page.setSelected(key, checked)
                }
                onCopyLinkRequested: {
                    if (clipboard.copyText(url)) {
                        page.copyNotice = qsTr("链接已复制")
                        copyNoticeTimer.restart()
                    }
                }
                onCopyPasswordRequested: {
                    if (clipboard.copyText(passCode)) {
                        page.copyNotice = qsTr("密码已复制")
                        copyNoticeTimer.restart()
                    }
                }
                onRevokeRequested: page.openRevoke([key])
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
    }

    KDialog {
        id: revokeDialog
        objectName: "revokeSharesDialog"
        title: qsTr("撤销分享")
        modal: true
        parent: Overlay.overlay
        anchors.centerIn: Overlay.overlay
        width: 400 + leftPadding + rightPadding
        closePolicy: page.actionRunning ? Popup.NoAutoClose : Popup.CloseOnEscape

        property string errorText: ""

        footer: DialogFooter {
            outerMargin: revokeDialog.padding
            okText: qsTr("撤销分享")
            okEnabled: !page.actionRunning
            cancelEnabled: !page.actionRunning
            onOkClicked: page.submitRevoke()
            onCancelClicked: revokeDialog.close()
        }

        onOpened: errorText = ""

        ColumnLayout {
            width: 400
            spacing: 8

            Label {
                Layout.fillWidth: true
                wrapMode: Text.Wrap
                text: qsTr("确定撤销选中的 %1 个分享链接吗？\n链接会立即失效，但不会删除网盘里的源文件。")
                    .arg(page.pendingIds.length)
            }

            Label {
                Layout.fillWidth: true
                wrapMode: Text.Wrap
                visible: text.length > 0
                text: revokeDialog.errorText
                font.pixelSize: theme.fontMeta
                color: theme.danger
            }
        }
    }

    function syncRows() {
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
        page.pruneSelection(wantedIds)
    }

    function normalize(share) {
        return {
            key: share.id,
            title: share.title,
            url: share.url,
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

    function isSelected(id) {
        return !!id && selectedMap[id] === true
    }

    function setSelected(id, checked) {
        if (!id)
            return
        const next = ({})
        const ids = Object.keys(selectedMap)
        for (let i = 0; i < ids.length; i++) {
            if (ids[i] !== id)
                next[ids[i]] = true
        }
        if (checked)
            next[id] = true
        selectedMap = next
    }

    function selectIds(ids) {
        const next = ({})
        for (let i = 0; i < (ids ? ids.length : 0); i++) {
            if (ids[i])
                next[ids[i]] = true
        }
        selectedMap = next
    }

    function clearSelection() {
        selectedMap = ({})
    }

    function selectedIds() {
        return Object.keys(selectedMap)
    }

    function toggleSelectAll() {
        if (allSelected) {
            clearSelection()
            return
        }
        const ids = []
        for (let i = 0; i < shareRows.count; i++)
            ids.push(shareRows.get(i).key)
        selectIds(ids)
    }

    function pruneSelection(present) {
        const next = ({})
        let changed = false
        const ids = Object.keys(selectedMap)
        for (let i = 0; i < ids.length; i++) {
            if (present[ids[i]])
                next[ids[i]] = true
            else
                changed = true
        }
        if (changed)
            selectedMap = next
    }

    function openRevoke(ids) {
        if (!backend || !ids || ids.length === 0 || actionRunning)
            return
        pendingIds = ids.slice()
        revokeDialog.errorText = ""
        revokeDialog.open()
    }

    function submitRevoke() {
        if (!backend || pendingIds.length === 0 || actionRunning)
            return
        revokeDialog.errorText = ""
        backend.revokeShares(pendingIds.slice())
    }
}
