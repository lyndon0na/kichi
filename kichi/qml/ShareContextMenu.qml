pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls

// 分享列表的右键菜单：单项和批量共用。
//
// 菜单父项固定为窗口 overlay，坐标由 SharesPage 在行上换算成窗口坐标后传入
// （见 docs/pitfalls.md 第 10 条）。
Item {
    id: root

    property var backend: null
    property var targetIds: []

    readonly property int targetCount: targetIds ? targetIds.length : 0

    signal copyRequested(var ids)
    signal revokeRequested(var ids)

    property alias menu: contextMenu

    Menu {
        id: contextMenu
        objectName: "shareContextMenu"
        parent: Overlay.overlay
        width: 238

        function openAt(point) {
            popup(Qt.point(point.x, point.y))
        }

        MenuItem {
            objectName: "shareCopyLinkItem"
            text: root.targetCount > 1
                  ? qsTr("复制选中的 %1 个链接").arg(root.targetCount)
                  : qsTr("复制链接")
            enabled: root.targetCount > 0
            onTriggered: root.copyRequested(root.targetIds.slice())
        }

        MenuItem {
            objectName: "shareRevokeItem"
            text: root.targetCount > 1
                  ? qsTr("取消选中的 %1 个分享…").arg(root.targetCount)
                  : qsTr("取消分享…")
            enabled: root.backend !== null && !root.backend.shareActionRunning
            onTriggered: root.revokeRequested(root.targetIds.slice())
        }
    }

    function openFor(ids, point) {
        if (!ids || ids.length === 0)
            return
        targetIds = ids.slice()
        contextMenu.openAt(point)
    }
}
