pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls

// 最近保存 / 播放历史 / 已加星标共用的右键菜单。
// 除星标外不修改源条目：改名、移动、复制与回收站仍只在文件页提供。
Item {
    id: root

    property var backend: null
    property var targetEntry: null
    property string targetId: ""
    property string targetName: ""
    property bool targetIsFolder: false
    property bool targetPlayable: false
    property bool targetStarred: false
    property bool targetPending: false
    property bool targetHasPartial: false
    property point targetPoint: Qt.point(0, 0)

    readonly property bool hasOpenAction: root.targetPlayable || root.targetIsFolder

    signal playRequested(string id, point position)
    signal openFolderRequested(string id)
    signal revealLocationRequested(string id)
    signal starRequested(string id, bool starred)
    signal downloadRequested(string id)
    signal shareRequested(var entry)
    signal refreshRequested()

    property alias menu: rowMenu
    property alias blankMenu: blankPopup

    Menu {
        id: rowMenu

        objectName: "libraryContextMenu"
        parent: Overlay.overlay
        width: 210

        function openAt(point) {
            popup(Qt.point(point.x, point.y))
        }

        MenuItem {
            objectName: "libraryPlayItem"
            text: qsTr("播放")
            visible: root.targetPlayable
            enabled: root.backend !== null && !root.backend.filesLoading
                     && root.backend.playMenu.length === 0
            onTriggered: root.playRequested(root.targetId, root.targetPoint)
        }

        MenuItem {
            objectName: "libraryOpenFolderItem"
            text: qsTr("打开文件夹")
            visible: root.targetIsFolder
            enabled: root.backend !== null && !root.backend.filesLoading
            onTriggered: root.openFolderRequested(root.targetId)
        }

        MenuItem {
            objectName: "libraryRevealItem"
            text: root.targetIsFolder
                  ? qsTr("打开目录所在位置")
                  : qsTr("打开文件所在位置")
            enabled: root.backend !== null && !root.backend.filesLoading
            onTriggered: root.revealLocationRequested(root.targetId)
        }

        MenuItem {
            objectName: "libraryStarItem"
            text: root.targetStarred ? qsTr("取消星标") : qsTr("加星标")
            enabled: root.backend !== null && !root.backend.fileActionRunning
                     && !root.backend.libraryLoading
            onTriggered: root.starRequested(root.targetId, !root.targetStarred)
        }

        MenuSeparator {
            visible: root.hasOpenAction
        }

        MenuItem {
            objectName: "libraryDownloadItem"
            text: {
                if (root.targetPending)
                    return qsTr("已在下载队列中，去查看")
                if (root.targetHasPartial)
                    return qsTr("继续下载")
                return root.targetIsFolder
                        ? qsTr("下载整个文件夹到本地")
                        : qsTr("下载到本地")
            }
            enabled: root.backend !== null
            onTriggered: root.downloadRequested(root.targetId)
        }

        MenuItem {
            objectName: "libraryShareItem"
            text: qsTr("分享…")
            enabled: root.backend !== null && !root.backend.fileActionRunning
                     && !root.backend.shareActionRunning
            onTriggered: root.shareRequested(root.targetEntry)
        }
    }

    Menu {
        id: blankPopup

        objectName: "libraryBlankMenu"
        parent: Overlay.overlay
        width: 160

        MenuItem {
            objectName: "libraryRefreshItem"
            text: qsTr("刷新")
            enabled: root.backend !== null && !root.backend.libraryLoading
                     && !root.backend.fileActionRunning
            onTriggered: root.refreshRequested()
        }
    }

    function openFor(entry, point) {
        if (!root.backend || !entry || !entry.id)
            return
        root.targetEntry = entry
        root.targetId = String(entry.id)
        root.targetName = String(entry.name || "")
        root.targetIsFolder = entry.kind === "folder"
        root.targetPlayable = entry.playable === true
        root.targetStarred = entry.starred === true
        root.targetPending = typeof root.backend.isDownloadPending === "function"
                             && root.backend.isDownloadPending(root.targetId)
        root.targetHasPartial = !root.targetIsFolder
                                && typeof root.backend.hasPartialDownload === "function"
                                && root.backend.hasPartialDownload(root.targetId)
        root.targetPoint = point
        rowMenu.openAt(point)
    }

    function openBlankAt(point) {
        if (!root.backend)
            return
        blankPopup.popup(Qt.point(point.x, point.y))
    }
}
