import QtQuick
import QtQuick.Controls

// 传输任务行的右键菜单。
//
// 行内仍保留最常用的动作；菜单按当前状态只显示可用项，移除记录只对顶层
// 已结束 / 已中断行开放。文件夹父行移除时会连同整批子记录一起移除。
Item {
    id: root

    property var backend: null
    property string targetKey: ""
    property string targetName: ""
    property string targetDirection: "download"
    property bool targetIsFolder: false
    property bool targetIsChild: false
    property int targetChildCount: 0
    property bool targetCanPause: false
    property bool targetCanResume: false
    property bool targetCanCancel: false
    property bool targetCanRetry: false
    property bool targetCanReveal: false
    property bool targetCanRemove: false

    readonly property bool backendReady: root.backend !== null
    readonly property bool hasMainActions: root.targetCanPause
                                           || root.targetCanResume
                                           || root.targetCanCancel
                                           || root.targetCanRetry
                                           || root.targetCanReveal

    signal pauseRequested(string key)
    signal resumeRequested(string key)
    signal cancelRequested(string key)
    signal retryRequested(string key)
    signal revealRequested(string key)
    signal removeRequested(string key, string name, bool isFolder, int childCount)

    function phaseActive(phase) {
        return phase === "queued" || phase === "preparing"
                || phase === "running" || phase === "pausing"
    }

    function phaseFinished(phase) {
        return phase === "done" || phase === "failed" || phase === "cancelled"
    }

    /// rowData 是 TransfersPage 规范化后的行对象，point 是窗口坐标。
    function openFor(rowData, point) {
        if (!rowData || rowData.phase === "notice")
            return

        var phase = String(rowData.phase || "")
        var active = root.phaseActive(phase)
        var pausing = phase === "pausing"
        var paused = phase === "paused"
        var failed = phase === "failed"
        var interrupted = phase === "interrupted"

        root.targetKey = String(rowData.key || "")
        root.targetName = String(rowData.name || "")
        root.targetDirection = String(rowData.direction || "download")
        root.targetIsFolder = rowData.folder === true
        root.targetIsChild = rowData.rowKind === "child"
        root.targetChildCount = Number(rowData.childCount || 0)
        var upload = root.targetDirection === "upload"
        root.targetCanPause = active && !pausing && !root.targetIsChild
                              && (!upload || phase === "queued")
        root.targetCanResume = paused && !root.targetIsChild
        root.targetCanCancel = (pausing || paused) ? !root.targetIsChild : active
        root.targetCanRetry = (failed || interrupted)
                              && (!root.targetIsFolder || root.targetChildCount === 0)
        root.targetCanReveal = String(rowData.path || "").length > 0
        root.targetCanRemove = !root.targetIsChild
                               && (root.phaseFinished(phase) || interrupted)

        contextMenu.openAt(point)
    }

    Menu {
        id: contextMenu
        objectName: "transferContextMenu"
        parent: Overlay.overlay
        width: 200

        function openAt(point) {
            popup(Qt.point(point.x, point.y))
        }

        MenuItem {
            objectName: "transferPauseItem"
            text: qsTr("暂停")
            visible: root.targetCanPause
            enabled: root.backendReady
            onTriggered: root.pauseRequested(root.targetKey)
        }

        MenuItem {
            objectName: "transferResumeItem"
            text: qsTr("继续")
            visible: root.targetCanResume
            enabled: root.backendReady
            onTriggered: root.resumeRequested(root.targetKey)
        }

        MenuItem {
            objectName: "transferCancelItem"
            text: qsTr("取消")
            visible: root.targetCanCancel
            enabled: root.backendReady
            onTriggered: root.cancelRequested(root.targetKey)
        }

        MenuItem {
            objectName: "transferRetryItem"
            text: qsTr("重试")
            visible: root.targetCanRetry
            enabled: root.backendReady
            onTriggered: root.retryRequested(root.targetKey)
        }

        MenuItem {
            objectName: "transferRevealItem"
            text: qsTr("打开位置")
            visible: root.targetCanReveal
            enabled: root.backendReady
            onTriggered: root.revealRequested(root.targetKey)
        }

        MenuSeparator {
            visible: root.hasMainActions && root.targetCanRemove
        }

        MenuItem {
            objectName: "transferRemoveItem"
            text: qsTr("移除记录…")
            visible: root.targetCanRemove
            enabled: root.backendReady
            onTriggered: root.removeRequested(root.targetKey, root.targetName,
                                              root.targetIsFolder,
                                              root.targetChildCount)
        }
    }
}
