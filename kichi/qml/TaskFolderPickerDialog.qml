pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 离线任务保存位置选择器。
//
// 复用移动 / 复制选择器的独立目录列表状态，只列云端文件夹。
// 当前层可以直接作为保存位置，也可以继续进入子文件夹。
KDialog {
    id: picker

    objectName: "taskFolderPickerDialog"
    property var backend: null
    property var startPath: []
    /// 根 → 当前，元素为 { id, name }
    property var path: []
    /// 确认时返回给离线任务弹窗的完整路径
    property var selectedPath: []
    property int seenRevision: -1
    /// 目录 id → 离开时的 contentY：进入子目录回顶部，上一级回到原位置
    property var scrollByDir: ({})
    /// 下一次列表就绪后要恢复的位置（< 0 回到顶部）；
    /// -2 表示未指定——同目录后台更新沿用当前列表位置，不跳顶部
    property real restoreY: -2

    readonly property int formWidth: 460
    /// 列表 + 提示条总高度固定：提示出现 / 消失时窗口和列表都不动
    readonly property int listHeight: 224
    readonly property int hintHeight: 36
    readonly property bool hasListError: backend !== null
                                          && backend.moveTargetsError !== ""
    UiMetrics {
        id: metrics
    }

    readonly property string rootName: {
        if (!backend || typeof backend.currentPath !== "function")
            return qsTr("全部文件")
        const raw = backend.currentPath() || []
        return raw.length > 0 && String(raw[0].name || "").length > 0
                ? String(raw[0].name) : qsTr("全部文件")
    }
    readonly property string targetId: path.length > 0
                                     ? String(path[path.length - 1].id || "") : ""
    readonly property string targetPathText: pathText(path)
    readonly property bool canChoose: backend !== null

    title: qsTr("选择保存位置")
    modal: true
    parent: Overlay.overlay
    anchors.centerIn: Overlay.overlay
    width: formWidth + leftPadding + rightPadding
    closePolicy: Popup.CloseOnEscape

    signal folderChosen(string folderId, string pathText)

    onClosed: {
        if (picker.backend)
            picker.backend.cancelMoveTargets()
    }

    function openPicker(initialPath) {
        if (!picker.backend)
            return
        const source = initialPath && initialPath.length > 0
                     ? initialPath : picker.startPath
        picker.path = normalizePath(source)
        picker.selectedPath = picker.path.slice()
        picker.seenRevision = -1
        picker.scrollByDir = ({})
        picker.restoreY = -1
        picker.open()
        picker.reload()
    }

    function normalizePath(raw) {
        const result = [{ id: "", name: picker.rootName }]
        if (!raw)
            return result
        for (let i = 0; i < raw.length; i++) {
            const id = String(raw[i].id || "")
            if (id.length === 0)
                continue
            result.push({ id: id, name: String(raw[i].name || id) })
        }
        return result
    }

    function pathText(items) {
        const parts = []
        for (let i = 0; i < items.length; i++)
            parts.push(String(items[i].name || ""))
        return parts.length > 0 ? parts.join(" / ") : picker.rootName
    }

    /// 记住当前目录列表的滚动位置（导航前调用）
    function rememberListScroll() {
        const next = ({})
        const keys = Object.keys(scrollByDir)
        for (let i = 0; i < keys.length; i++)
            next[keys[i]] = scrollByDir[keys[i]]
        next[targetId] = folderList.contentY
        scrollByDir = next
    }

    function reload() {
        folderList.beginDirectoryLoad()
        if (picker.backend)
            picker.backend.loadMoveTargets(picker.targetId, false)
    }

    /// 手动刷新：跳过本地快照强制联网，刷新后留在原位置
    function refresh() {
        if (!picker.backend || picker.backend.moveTargetsLoading)
            return
        folderList.beginDirectoryLoad()
        picker.backend.loadMoveTargets(picker.targetId, true)
    }

    function goUp() {
        if (path.length <= 1)
            return
        rememberListScroll()
        path = path.slice(0, path.length - 1)
        const saved = scrollByDir[targetId]
        restoreY = saved !== undefined ? saved : -1
        reload()
    }

    function enterFolder(folder) {
        if (!folder)
            return
        rememberListScroll()
        restoreY = -1
        path = path.concat([{
            id: String(folder.id || ""),
            name: String(folder.name || folder.id || "")
        }])
        reload()
    }

    function chooseHere() {
        if (!canChoose)
            return
        selectedPath = path.slice()
        folderChosen(targetId, targetPathText)
        close()
    }

    ListModel {
        id: foldersModel
    }

    function syncFolders() {
        if (!backend || backend.moveTargetsRevision === seenRevision)
            return
        seenRevision = backend.moveTargetsRevision
        const pending = picker.restoreY
        picker.restoreY = -2
        // 同目录的后台补全不能把用户刚滚到的位置冲回顶部；
        // 位置必须在回顶之前取，否则拿到的已经是 0
        const restore = pending === -2 ? folderList.contentY : pending
        // 列表缩短时 Qt 会残留旧 originY，替换模型前先回顶
        folderList.prepareDirectoryReplace()
        foldersModel.clear()
        const list = backend.moveTargets || []
        for (let i = 0; i < list.length; i++)
            foldersModel.append({
                folderId: String(list[i].id || ""),
                folderName: String(list[i].name || "")
            })
        folderList.finishDirectoryLoad(restore)
    }

    Connections {
        target: picker.backend

        function onChanged() {
            picker.syncFolders()
        }
    }

    ColumnLayout {
        width: picker.formWidth
        spacing: picker.theme.gapSm

        Label {
            Layout.fillWidth: true
            wrapMode: Text.Wrap
            text: qsTr("选择离线任务下载完成后的保存文件夹。")
            color: picker.theme.textPrimary
            font.pixelSize: picker.theme.fontBody
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: picker.theme.gapXs

            KButton {
                objectName: "taskFolderPickerUp"
                text: qsTr("↑ 上一级")
                variant: "secondary"
                implicitHeight: 30
                enabled: picker.backend !== null && picker.path.length > 1
                onClicked: picker.goUp()
            }

            ElidedLabel {
                objectName: "taskFolderPickerPath"
                Layout.fillWidth: true
                elide: Text.ElideMiddle
                text: picker.targetPathText
                color: picker.theme.accent
            }

            BusyIndicator {
                visible: picker.backend !== null && picker.backend.moveTargetsLoading
                         && foldersModel.count === 0
                running: visible
                Layout.preferredWidth: 18
                Layout.preferredHeight: 18
            }

            KButton {
                objectName: "taskFolderPickerRefresh"
                text: qsTr("刷新")
                variant: "secondary"
                implicitHeight: 30
                enabled: picker.backend !== null && !picker.backend.moveTargetsLoading
                onClicked: picker.refresh()
            }
        }

        FolderPickerListView {
            id: folderList

            objectName: "taskFolderPickerList"
            Layout.fillWidth: true
            Layout.preferredHeight: picker.listHeight
            model: foldersModel
            enabled: picker.backend !== null

            onRowActivated: function (index) {
                const item = foldersModel.get(index)
                if (item)
                    picker.enterFolder({
                        id: item.folderId,
                        name: item.folderName
                    })
            }

            delegate: ItemDelegate {
                required property string folderId
                required property string folderName

                width: folderList.width
                text: folderName
                onClicked: picker.enterFolder({
                    id: folderId,
                    name: folderName
                })
            }
        }

        // 固定高度的提示条：空目录 / 错误都放这里，避免弹窗和列表跳动
        Item {
            Layout.fillWidth: true
            Layout.preferredHeight: picker.hintHeight

            Label {
                objectName: "taskFolderPickerError"
                anchors.left: parent.left
                anchors.right: retry.visible ? retry.left : parent.right
                anchors.rightMargin: retry.visible ? picker.theme.gapSm : 0
                anchors.verticalCenter: parent.verticalCenter
                wrapMode: Text.Wrap
                maximumLineCount: 2
                elide: Text.ElideRight
                color: picker.theme.danger
                font.pixelSize: picker.theme.fontMeta
                visible: picker.hasListError
                text: picker.backend ? picker.backend.moveTargetsError : ""
            }

            Label {
                objectName: "taskFolderPickerEmpty"
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                wrapMode: Text.Wrap
                maximumLineCount: 2
                elide: Text.ElideRight
                color: picker.theme.textMuted
                font.pixelSize: picker.theme.fontMeta
                visible: picker.backend !== null && !picker.hasListError
                         && !picker.backend.moveTargetsLoading
                         && foldersModel.count === 0
                text: qsTr("这个文件夹下没有子文件夹；可以直接保存到这里。")
            }

            KButton {
                id: retry
                objectName: "taskFolderPickerRetry"
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                text: qsTr("重试")
                variant: "secondary"
                implicitHeight: 28
                visible: picker.hasListError
                onClicked: picker.refresh()
            }
        }
    }

    footer: DialogFooter {
        outerMargin: picker.padding
        okText: qsTr("保存到这里")
        okEnabled: picker.canChoose
        onOkClicked: picker.chooseHere()
        onCancelClicked: picker.close()
    }
}
