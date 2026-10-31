import QtQuick
import QtQuick.Controls

// 离线任务行右键菜单：打开产物文件夹、定位产物所在目录、播放，以及删除。
Item {
    id: root

    property var backend: null
    property string targetId: ""
    property string targetName: ""
    property string targetFileId: ""
    property bool targetCanOpenFolder: false
    property bool targetCanRevealLocation: false
    property bool targetIsFolder: false
    property bool targetCanPlay: false
    property point targetPoint: Qt.point(0, 0)

    signal deleteRequested(string id, string name)
    signal openFolderRequested(string fileId)
    signal revealLocationRequested(string fileId)
    signal playRequested(string fileId, point position)

    Menu {
        id: contextMenu
        objectName: "taskContextMenu"
        parent: Overlay.overlay
        width: 180

        function openAt(point) {
            popup(Qt.point(point.x, point.y))
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
            objectName: "taskPlayItem"
            text: qsTr("播放（mpv）")
            visible: root.targetCanPlay
            enabled: root.backend !== null && root.targetFileId.length > 0
            onTriggered: root.playRequested(root.targetFileId, root.targetPoint)
        }

        MenuSeparator {
            visible: root.targetCanOpenFolder || root.targetCanRevealLocation
                     || root.targetCanPlay
        }

        MenuItem {
            objectName: "taskDeleteItem"
            text: qsTr("删除任务…")
            enabled: root.backend !== null && root.targetId.length > 0
            onTriggered: root.deleteRequested(root.targetId, root.targetName)
        }
    }

    function openFor(id, name, point, fileId, kind, phase, playable) {
        targetId = String(id || "")
        targetName = String(name || "")
        targetFileId = String(fileId || "")
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
        targetPoint = point
        contextMenu.openAt(point)
    }
}
