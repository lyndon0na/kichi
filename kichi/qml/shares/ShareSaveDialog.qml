pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "../components"
import "../core"
import "../dialogs"

// 转存分享条目时选择目标文件夹：复用移动 / 复制的目录选择器数据源，
// 但不需要「源文件夹不可选」的限制（分享条目不在自己的网盘里）。
KDialog {
    id: picker
    objectName: "shareSaveDialog"

    property var backend: null
    /// 已选中的分享条目 id
    property var sourceIds: []
    /// 目标路径：根 → 当前目标，元素为 { id, name }
    property alias path: nav.path
    /// 转存失败原因（由宿主弹窗在 shareRestoreDone 里写入）
    property string errorText: ""

    readonly property bool running: backend ? backend.shareSaveRunning : false
    readonly property string targetId: nav.targetId
    readonly property string targetLabel: path.length > 0 ? path[path.length - 1].name
                                                          : qsTr("全部文件")
    readonly property bool canApply: backend !== null && !running
                                     && sourceIds.length > 0
    readonly property int formWidth: 460
    readonly property int listHeight: 224
    readonly property int hintHeight: 36
    readonly property bool hasListError: backend !== null
                                          && backend.moveTargetsError !== ""

    title: sourceIds.length > 1
           ? qsTr("转存 %1 项到…").arg(sourceIds.length)
           : qsTr("转存到…")
    modal: true
    parent: Overlay.overlay
    anchors.centerIn: Overlay.overlay
    width: formWidth + leftPadding + rightPadding
    closePolicy: running ? Popup.NoAutoClose : Popup.CloseOnEscape

    onClosed: {
        if (picker.backend)
            picker.backend.cancelMoveTargets()
    }

    FolderPickerNavigation {
        id: nav
        backend: picker.backend
        fileList: folderList
    }

    function openFor(ids) {
        if (!backend || !ids || ids.length === 0)
            return
        picker.sourceIds = ids.slice()
        picker.errorText = ""
        picker.path = [{ id: "", name: qsTr("全部文件") }]
        nav.reset()
        picker.open()
        nav.reload()
    }

    function refresh() {
        if (!backend || backend.moveTargetsLoading || running)
            return
        errorText = ""
        nav.refresh()
    }

    function goUp() {
        if (path.length <= 1 || running)
            return
        errorText = ""
        nav.goUp()
    }

    function enterFolder(folder) {
        if (!folder || running)
            return
        errorText = ""
        nav.enterFolder(folder)
    }

    function apply() {
        if (!canApply)
            return
        errorText = ""
        // 接受后保持打开：成功由宿主弹窗关闭，失败原因写回 errorText。
        picker.backend.restoreShareFiles(picker.sourceIds, picker.targetId)
    }

    ColumnLayout {
        width: picker.formWidth
        spacing: picker.theme.gapSm

        Label {
            Layout.fillWidth: true
            wrapMode: Text.Wrap
            maximumLineCount: 1
            elide: Text.ElideRight
            text: picker.sourceIds.length > 1
                  ? qsTr("把选中的 %1 项转存到：").arg(picker.sourceIds.length)
                  : qsTr("把选中的条目转存到：")
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: picker.theme.gapXs

            KButton {
                objectName: "shareSaveUp"
                text: qsTr("↑ 上一级")
                variant: "secondary"
                implicitHeight: 30
                enabled: picker.backend !== null && picker.path.length > 1
                         && !picker.running
                onClicked: picker.goUp()
            }

            ElidedLabel {
                objectName: "shareSaveTarget"
                Layout.fillWidth: true
                elide: Text.ElideMiddle
                text: picker.targetLabel
                color: palette.link
            }

            BusyIndicator {
                visible: picker.backend !== null && picker.backend.moveTargetsLoading
                         && nav.foldersModel.count === 0
                running: visible
                Layout.preferredWidth: 18
                Layout.preferredHeight: 18
            }

            KButton {
                objectName: "shareSaveRefresh"
                text: qsTr("刷新")
                variant: "secondary"
                implicitHeight: 30
                enabled: picker.backend !== null && !picker.backend.moveTargetsLoading
                         && !picker.running
                onClicked: picker.refresh()
            }
        }

        FolderPickerListView {
            id: folderList

            objectName: "shareSaveFolderList"
            Layout.fillWidth: true
            Layout.preferredHeight: picker.listHeight
            model: nav.foldersModel
            enabled: picker.backend !== null && !picker.running

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

                width: folderList.width - 10
                text: folderName
                onClicked: picker.enterFolder({ id: folderId, name: folderName })
            }
        }

        // 固定高度的提示条，避免状态切换时弹窗高度跳动。
        Item {
            Layout.fillWidth: true
            Layout.preferredHeight: picker.hintHeight

            Label {
                objectName: "shareSaveError"
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                wrapMode: Text.Wrap
                maximumLineCount: 2
                elide: Text.ElideRight
                font.pixelSize: picker.theme.fontMeta
                color: picker.theme.danger
                visible: picker.errorText.length > 0
                text: picker.errorText
            }

            Label {
                objectName: "shareSaveListError"
                anchors.left: parent.left
                anchors.right: retry.visible ? retry.left : parent.right
                anchors.rightMargin: retry.visible ? picker.theme.gapSm : 0
                anchors.verticalCenter: parent.verticalCenter
                wrapMode: Text.Wrap
                maximumLineCount: 2
                elide: Text.ElideRight
                font.pixelSize: picker.theme.fontMeta
                color: picker.theme.danger
                visible: picker.errorText.length === 0 && picker.hasListError
                text: picker.backend ? picker.backend.moveTargetsError : ""
            }

            KButton {
                id: retry
                objectName: "shareSaveRetry"
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                text: qsTr("重试")
                variant: "secondary"
                implicitHeight: 28
                visible: picker.errorText.length === 0 && picker.hasListError
                onClicked: picker.refresh()
            }

            Label {
                objectName: "shareSaveNote"
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                wrapMode: Text.Wrap
                maximumLineCount: 2
                elide: Text.ElideRight
                font.pixelSize: picker.theme.fontMeta
                color: picker.theme.textMuted
                visible: picker.errorText.length === 0 && !picker.hasListError
                         && picker.running
                text: qsTr("正在转存，完成后会出现在这个文件夹里…")
            }

            Label {
                objectName: "shareSaveEmpty"
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                wrapMode: Text.Wrap
                maximumLineCount: 2
                elide: Text.ElideRight
                font.pixelSize: picker.theme.fontMeta
                color: picker.theme.textMuted
                visible: picker.errorText.length === 0 && !picker.hasListError
                         && !picker.running
                         && picker.backend !== null && !picker.backend.moveTargetsLoading
                         && nav.foldersModel.count === 0
                text: qsTr("这个文件夹下没有子文件夹；可以直接转存到这里。")
            }
        }
    }

    footer: DialogFooter {
        outerMargin: picker.padding
        okText: qsTr("转存到这里")
        okEnabled: picker.canApply
        cancelEnabled: !picker.running
        onOkClicked: picker.apply()
        onCancelClicked: picker.close()
    }
}
