pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "../components"
import "../core"
import "../dialogs"

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
    property alias path: nav.path
    /// 确认时返回给离线任务弹窗的完整路径
    property var selectedPath: []

    readonly property int formWidth: 460
    /// 列表 + 提示条总高度固定：提示出现 / 消失时窗口和列表都不动
    readonly property int listHeight: 224
    readonly property int hintHeight: 36
    readonly property bool hasListError: backend !== null
                                          && backend.moveTargetsError !== ""
    UiMetrics {
        id: metrics
    }

    FolderPickerNavigation {
        id: nav
        backend: picker.backend
        fileList: folderList
    }

    readonly property string rootName: {
        if (!backend || typeof backend.currentPath !== "function")
            return qsTr("全部文件")
        const raw = backend.currentPath() || []
        return raw.length > 0 && String(raw[0].name || "").length > 0
                ? String(raw[0].name) : qsTr("全部文件")
    }
    readonly property string targetId: nav.targetId
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
        nav.reset()
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
        nav.reload()
    }

    /// 手动刷新：跳过本地快照强制联网，刷新后留在原位置
    function refresh() {
        if (!picker.backend || picker.backend.moveTargetsLoading)
            return
        nav.refresh()
    }

    function goUp() {
        nav.goUp()
    }

    function enterFolder(folder) {
        nav.enterFolder(folder)
    }

    function chooseHere() {
        if (!canChoose)
            return
        selectedPath = path.slice()
        folderChosen(targetId, targetPathText)
        close()
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
                         && nav.foldersModel.count === 0
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
            model: nav.foldersModel
            enabled: picker.backend !== null

            onRowActivated: function (index) {
                const item = nav.foldersModel.get(index)
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
                         && nav.foldersModel.count === 0
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
