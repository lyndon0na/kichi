pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 文件条目的右键菜单 + 空白处右键菜单。
//
// 弹窗（重命名 / 新建 / 回收站）在 FileActionDialogs.qml：
// 菜单只管「点哪一项」，结果怎么显示由那边负责。
//
// 用法：放一个实例，调 openContext(entry, point, ids, fromSearch) 弹条目菜单；
// 调 openBlankAt(point) 弹空白菜单。
Item {
    id: root

    property var backend: null

    // 当前操作的条目（右键菜单打开时记下）
    property var target: null
    property string targetId: ""
    property string targetName: ""
    property bool targetIsFolder: false
    property bool targetPlayable: false
    property bool targetStarred: false
    // 这个条目是否已在下载队列里（打开菜单时问一次后端；见 openContext）
    property bool targetPending: false
    // 这个文件是否有可续传的断点（同一次打开菜单时问一次）
    property bool targetHasPartial: false
    // 右键处的位置（**窗口**坐标系）：清晰度菜单从这里弹出
    property point targetPoint: Qt.point(0, 0)
    // 右键时的整个选择（多选批量操作据此计数）
    property var targetIds: []
    readonly property int targetCount: targetIds.length
    // 搜索结果里的条目没有「源目录」上下文，暂不提供移动 / 复制
    property bool fromSearch: false

    // 菜单里点「播放」：页面把位置交给清晰度菜单，再让后端取档位
    signal playRequested(point anchor)
    signal moveRequested(var entry, var ids)
    signal copyRequested(var entry, var ids)
    signal shareRequested(var entry, var ids)
    // 空白菜单的「刷新」交给页面：页面要先记住滚动位置
    signal refreshRequested()

    FileActionDialogs {
        id: dialogs
        backend: root.backend
    }

    // —— 条目右键菜单 ——
    Menu {
        id: contextMenu

        // 父项固定成窗口 overlay：Popup 的坐标是相对父项的，不固定就会跟着页面偏移
        // （与清晰度菜单同一条纪律，见 docs/pitfalls.md 第 10 条）
        parent: Overlay.overlay

        // 宽度只在这里定，子项跟着撑满：
        // 每个 MenuItem 各自设 width 会比菜单容器还宽，高亮条就溢出去了
        width: 210

        // 在窗口坐标系里 point 处弹出（左上角对准鼠标），和系统右键菜单一样。
        // 用 popup(Qt.point) 绝对定位，不用 x / y：那套坐标是相对本菜单父项的
        // （父项已固定为 overlay，见上）。超出窗口由 Menu 自己的 margins 夹回。
        function openAt(point) {
            popup(Qt.point(point.x, point.y))
        }

        // 只有视频/音频才有播放入口；多选时只对单条开放
        MenuItem {
            objectName: "playItem"
            text: qsTr("播放")
            visible: root.targetPlayable && root.targetCount === 1
            enabled: root.backend !== null && !root.backend.filesLoading
                     && root.backend.playMenu.length === 0
            onTriggered: {
                // targetPoint 已经是窗口坐标，直接交给清晰度菜单
                root.playRequested(root.targetPoint)
                if (root.backend)
                    root.backend.requestPlay(root.targetId)
            }
        }

        // 下载到本地（文件直接下；文件夹递归下，保留层级）。
        // 多选时每个条目各自一批、按顺序排队（一次仍只跑一批）；跨批次不合并通知
        // （docs/adr/0010）。已经在队列里的条目由后端跳过，不重复登记。
        MenuItem {
            objectName: "downloadItem"
            text: {
                if (root.targetCount > 1)
                    return qsTr("下载到本地（%1 项）").arg(root.targetCount)
                if (root.targetPending)
                    return qsTr("已在下载队列中，去查看")
                if (root.targetHasPartial)
                    return qsTr("继续下载")
                return root.targetIsFolder ? qsTr("下载整个文件夹到本地") : qsTr("下载到本地")
            }
            enabled: root.backend !== null
            onTriggered: {
                if (!root.backend)
                    return
                if (root.targetCount > 1)
                    root.backend.downloadEntries(root.targetIds)
                else
                    root.backend.downloadEntry(root.targetId)
            }
        }

        MenuItem {
            objectName: "starItem"
            text: {
                if (root.targetCount > 1) {
                    return root.targetStarred
                            ? qsTr("取消选中的 %1 项星标").arg(root.targetCount)
                            : qsTr("为选中的 %1 项加星标").arg(root.targetCount)
                }
                return root.targetStarred ? qsTr("取消星标") : qsTr("加星标")
            }
            enabled: root.backend !== null && !root.backend.filesLoading
                     && !root.backend.fileActionRunning
            onTriggered: {
                if (root.backend)
                    root.backend.setStarred(root.targetIds, !root.targetStarred)
            }
        }

        MenuItem {
            objectName: "shareItem"
            text: root.targetCount > 1
                  ? qsTr("分享选中的 %1 项…").arg(root.targetCount)
                  : qsTr("分享…")
            enabled: root.backend !== null && !root.backend.filesLoading
                     && !root.backend.fileActionRunning
                     && !root.backend.shareActionRunning
            onTriggered: root.shareRequested(root.target, root.targetIds)
        }

        MenuSeparator {
            visible: !root.fromSearch
        }

        MenuItem {
            text: root.targetCount > 1
                  ? qsTr("移动到…（%1 项）").arg(root.targetCount)
                  : qsTr("移动到…")
            visible: !root.fromSearch
            enabled: root.backend !== null && !root.backend.filesLoading
                     && !root.backend.fileActionRunning
            onTriggered: root.moveRequested(root.target, root.targetIds)
        }

        MenuItem {
            text: root.targetCount > 1
                  ? qsTr("复制到…（%1 项）").arg(root.targetCount)
                  : qsTr("复制到…")
            visible: !root.fromSearch
            enabled: root.backend !== null && !root.backend.filesLoading
                     && !root.backend.fileActionRunning
            onTriggered: root.copyRequested(root.target, root.targetIds)
        }

        MenuSeparator {}

        MenuItem {
            text: qsTr("重命名…")
            enabled: root.backend !== null && !root.backend.filesLoading
                     && !root.backend.fileActionRunning
                     && root.targetCount === 1
            onTriggered: dialogs.openRename(root.target)
        }

        MenuItem {
            text: root.targetCount > 1
                  ? qsTr("移至回收站…（%1 项）").arg(root.targetCount)
                  : qsTr("移至回收站…")
            enabled: root.backend !== null && !root.backend.filesLoading
                     && !root.backend.fileActionRunning
            onTriggered: dialogs.openTrash(root.target, root.targetIds)
        }
    }

    // —— 空白处右键菜单（列表没有条目时也能新建 / 刷新）——
    Menu {
        id: blankMenu
        parent: Overlay.overlay
        width: 180

        MenuItem {
            text: qsTr("新建文件夹…")
            enabled: root.backend !== null && !root.backend.filesLoading
                     && !root.backend.fileActionRunning
            onTriggered: dialogs.openNewFolder()
        }

        MenuItem {
            text: qsTr("刷新")
            enabled: root.backend !== null && !root.backend.filesLoading
            onTriggered: root.refreshRequested()
        }
    }

    // —— 入口 ——

    function openContext(entry, point, ids, fromSearch) {
        if (!entry)
            return
        target = entry
        targetId = entry.id
        targetName = entry.name
        targetIsFolder = entry.kind === "folder"
        targetPlayable = entry.playable === true
        targetStarred = entry.starred === true
        // 每次打开都重新问一次：条目可能刚下完（已释放）或刚被排上
        targetPending = root.backend !== null && root.backend.isDownloadPending(entry.id)
        // 文件夹没有「单文件断点」；只有文件才可能显示「继续下载」
        targetHasPartial = !targetIsFolder && root.backend !== null
                           && root.backend.hasPartialDownload(entry.id)
        targetPoint = point
        targetIds = ids && ids.length > 0 ? ids.slice() : [entry.id]
        root.fromSearch = fromSearch === true
        contextMenu.openAt(point)
    }

    function openBlankAt(point) {
        if (!root.backend)
            return
        blankMenu.popup(Qt.point(point.x, point.y))
    }

    /// 快捷键 F2：直接打开改名弹窗（不经过右键菜单）
    function openRenameFor(entry, ids) {
        if (!entry)
            return
        target = entry
        targetId = entry.id
        targetName = entry.name
        targetIsFolder = entry.kind === "folder"
        targetIds = ids && ids.length > 0 ? ids.slice() : [entry.id]
        dialogs.openRename(entry)
    }

    /// 快捷键 Delete：直接打开回收站确认（可多选）
    function openTrashFor(entry, ids) {
        if (!entry)
            return
        target = entry
        targetId = entry.id
        targetName = entry.name
        targetIsFolder = entry.kind === "folder"
        targetIds = ids && ids.length > 0 ? ids.slice() : [entry.id]
        dialogs.openTrash(entry, root.targetIds)
    }

    function openNewFolder() {
        dialogs.openNewFolder()
    }
}
