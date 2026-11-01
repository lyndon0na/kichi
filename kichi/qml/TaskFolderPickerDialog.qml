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

    readonly property int formWidth: 460
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
                                      && !backend.moveTargetsLoading

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

    function reload() {
        folderList.beginDirectoryLoad()
        if (picker.backend)
            picker.backend.loadMoveTargets(picker.targetId)
    }

    function goUp() {
        if (path.length <= 1)
            return
        path = path.slice(0, path.length - 1)
        reload()
    }

    function enterFolder(folder) {
        if (!folder)
            return
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
        foldersModel.clear()
        const list = backend.moveTargets || []
        for (let i = 0; i < list.length; i++)
            foldersModel.append({
                folderId: String(list[i].id || ""),
                folderName: String(list[i].name || "")
            })
        folderList.finishDirectoryLoad()
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
                enabled: picker.backend !== null
                         && !picker.backend.moveTargetsLoading
                         && picker.path.length > 1
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
                running: visible
                Layout.preferredWidth: 18
                Layout.preferredHeight: 18
            }
        }

        FolderPickerListView {
            id: folderList

            objectName: "taskFolderPickerList"
            Layout.fillWidth: true
            Layout.preferredHeight: 260
            model: foldersModel
            enabled: picker.backend !== null
                     && !picker.backend.moveTargetsLoading

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

                width: folderList.width - metrics.scrollBarSlot
                text: folderName
                onClicked: picker.enterFolder({
                    id: folderId,
                    name: folderName
                })
            }
        }

        Label {
            objectName: "taskFolderPickerEmpty"
            Layout.fillWidth: true
            wrapMode: Text.Wrap
            color: picker.theme.textMuted
            visible: picker.backend !== null && !picker.backend.moveTargetsLoading
                     && foldersModel.count === 0
                     && picker.backend.moveTargetsError === ""
            text: qsTr("这个文件夹下没有子文件夹；可以直接保存到这里。")
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: picker.theme.gapXs
            visible: picker.backend !== null
                     && picker.backend.moveTargetsError !== ""

            Label {
                objectName: "taskFolderPickerError"
                Layout.fillWidth: true
                wrapMode: Text.Wrap
                color: picker.theme.danger
                font.pixelSize: picker.theme.fontMeta
                text: picker.backend ? picker.backend.moveTargetsError : ""
            }

            KButton {
                objectName: "taskFolderPickerRetry"
                text: qsTr("重试")
                variant: "secondary"
                implicitHeight: 28
                onClicked: picker.reload()
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
