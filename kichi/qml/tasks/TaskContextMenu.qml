import QtQuick
import QtQuick.Controls

// 离线任务行右键菜单：打开产物文件夹、定位产物所在目录、播放，以及删除。
Item {
    id: root

    property var backend: null
    property string targetId: ""
    property string targetName: ""
    property string targetFileId: ""
    property string targetKind: ""
    property bool targetCanOpenFolder: false
    property bool targetCanRevealLocation: false
    property bool targetIsFolder: false
    property bool targetCanPlay: false
    // 只有已完成的任务才有产物，才能看产物的属性
    property bool targetHasProduct: false
    property bool targetCanRetry: false
    property point targetPoint: Qt.point(0, 0)

    signal deleteRequested(string id, string name)
    signal openFolderRequested(string fileId)
    signal revealLocationRequested(string fileId)
    signal playRequested(string fileId, point position)
    signal propertiesRequested(string fileId, string name, string kind)
    signal retryRequested(string id)

    Menu {
        id: contextMenu
        objectName: "taskContextMenu"
        parent: Overlay.overlay
        width: 180

        function openAt(point) {
            popup(Qt.point(point.x, point.y))
        }

        // 可播放媒体优先放第一项，减少右键后移动鼠标的距离。
        MenuItem {
            objectName: "taskPlayItem"
            text: qsTr("播放")
            visible: root.targetCanPlay
            enabled: root.backend !== null && root.targetFileId.length > 0
            onTriggered: root.playRequested(root.targetFileId, root.targetPoint)
        }

        MenuItem {
            objectName: "taskOpenFolderItem"
            text: qsTr("打开文件夹")
            visible: root.targetCanOpenFolder
            enabled: root.backend !== null && root.targetFileId.length > 0
            onTriggered: root.openFolderRequested(root.targetFileId)
        }

        MenuItem {
            objectName: "taskRevealLocationItem"
            text: root.targetIsFolder
                  ? qsTr("打开目录所在位置")
                  : qsTr("打开文件所在位置")
            visible: root.targetCanRevealLocation
            enabled: root.backend !== null && root.targetFileId.length > 0
            onTriggered: root.revealLocationRequested(root.targetFileId)
        }

        MenuItem {
            objectName: "taskRetryItem"
            text: qsTr("重新提交…")
            visible: root.targetCanRetry
            enabled: root.backend !== null && root.targetId.length > 0
            onTriggered: root.retryRequested(root.targetId)
        }

        MenuSeparator {
            visible: root.targetCanOpenFolder || root.targetCanRevealLocation
                     || root.targetCanPlay || root.targetCanRetry
                     || root.targetHasProduct
        }

        // 只看产物本身的信息：任务进度 / 磁力链接之类不在属性里重复。
        MenuItem {
            objectName: "taskPropertiesItem"
            text: qsTr("属性…")
            visible: root.targetHasProduct
            enabled: root.backend !== null
            onTriggered: root.propertiesRequested(root.targetFileId,
                                                  root.targetName,
                                                  root.targetKind)
        }

        MenuItem {
            objectName: "taskDeleteItem"
            text: qsTr("删除任务…")
            enabled: root.backend !== null && root.targetId.length > 0
            onTriggered: root.deleteRequested(root.targetId, root.targetName)
        }
    }

    function openFor(id, name, point, fileId, kind, phase, playable, retryable) {
        targetId = String(id || "")
        targetName = String(name || "")
        targetFileId = String(fileId || "")
        targetKind = String(kind || "")
        targetIsFolder = kind === "folder"
        targetCanOpenFolder = phase === "completed"
                              && targetIsFolder
                              && targetFileId.length > 0
        targetCanRevealLocation = phase === "completed"
                                  && (targetIsFolder || kind === "file")
                                  && targetFileId.length > 0
        targetCanPlay = phase === "completed"
                        && playable === true
                        && targetFileId.length > 0
        targetHasProduct = phase === "completed" && targetFileId.length > 0
        targetCanRetry = retryable === true
        targetPoint = point
        contextMenu.openAt(point)
    }
}
