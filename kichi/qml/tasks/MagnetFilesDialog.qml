pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "../components"

// 磁链解析结果：勾选要秒传的文件，并选择保存位置。
//
// 清单由 backend 的 resource/list 提供；这里只负责选择。点「保存选中」后
// 立即关闭弹窗，秒传在后台继续，进度与结果都显示在底栏。
KDialog {
    id: dialog

    objectName: "magnetFilesDialog"
    property var backend: null
    /// 空串表示根目录；秒传必须落到一个具体父目录。
    property string targetFolderId: ""
    property string targetFolderPath: ""
    property var pickerStartPath: []
    /// 按磁链里的目录层级保存；关闭后所有选中文件直接平铺到目标目录。
    property bool keepStructure: true
    /// 本次关闭由「保存选中」触发：关闭后要同时收起新任务弹窗。
    property bool saveRequested: false
    readonly property int checkedCount: fileTree.checkedCount
    readonly property int availableCount: fileTree.availableCount
    readonly property int totalFiles: fileTree.totalFiles

    signal saved()

    readonly property int listHeight: 300
    readonly property bool allChecked: fileTree.allChecked
    readonly property bool canSave: backend !== null && backend.magnetSaving !== true
                                    && fileTree.checkedCount > 0
    readonly property string destinationText: targetFolderPath.length > 0
                                              ? targetFolderPath : qsTr("全部文件")
    readonly property string summaryText: backend !== null
                                          ? String(backend.magnetSummary || "") : ""

    title: qsTr("选择要保存的文件")
    modal: true
    parent: Overlay.overlay
    anchors.centerIn: Overlay.overlay
    width: 560 + leftPadding + rightPadding

    onClosed: {
        const requested = saveRequested
        saveRequested = false
        fileTree.clear()
        if (requested)
            saved()
    }

    footer: DialogFooter {
        outerMargin: dialog.padding
        okText: qsTr("保存选中")
        okEnabled: dialog.canSave
        onOkClicked: dialog.save()
        onCancelClicked: dialog.close()
    }

    ColumnLayout {
        width: 560
        spacing: theme.gapSm

        ElidedLabel {
            objectName: "magnetRootName"
            Layout.fillWidth: true
            elide: Text.ElideMiddle
            text: dialog.backend !== null
                  ? String(dialog.backend.magnetRootName || qsTr("磁链文件"))
                  : qsTr("磁链文件")
            color: theme.textPrimary
            font.pixelSize: theme.fontSection
            font.weight: Font.DemiBold
        }

        Label {
            objectName: "magnetSummary"
            Layout.fillWidth: true
            wrapMode: Text.Wrap
            visible: text.length > 0
            text: dialog.summaryText
            color: theme.textMuted
            font.pixelSize: theme.fontMeta
        }

        // —— 选择态 ——
        RowLayout {
            Layout.fillWidth: true
            spacing: theme.gapSm

            NavIcon {
                implicitWidth: 16
                implicitHeight: 16
                Layout.alignment: Qt.AlignVCenter
                name: "folder"
                color: theme.textSecondary
            }

            ElidedLabel {
                objectName: "magnetDestinationPath"
                Layout.fillWidth: true
                elide: Text.ElideMiddle
                text: qsTr("保存到：%1").arg(dialog.destinationText)
                color: theme.textSecondary
                font.pixelSize: theme.fontMeta
                verticalAlignment: Text.AlignVCenter
            }

            KButton {
                objectName: "magnetChangeDestination"
                text: qsTr("更改位置…")
                variant: "ghost"
                implicitHeight: 28
                showTextToolTip: false
                onClicked: folderPicker.openPicker(dialog.pickerStartPath)
            }
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: theme.gapSm

            KButton {
                objectName: "magnetSelectAll"
                text: dialog.allChecked ? qsTr("全不选") : qsTr("全选")
                variant: "ghost"
                implicitHeight: 24
                leftPadding: 8
                rightPadding: 8
                enabled: dialog.availableCount > 0
                onClicked: fileTree.setAllChecked(!fileTree.allChecked)
            }

            Label {
                objectName: "magnetSelectionCount"
                Layout.fillWidth: true
                horizontalAlignment: Text.AlignRight
                text: qsTr("已选 %1 / %2").arg(fileTree.checkedCount).arg(fileTree.totalFiles)
                color: theme.textMuted
                font.pixelSize: theme.fontMeta
            }

            CheckBox {
                id: keepStructureCheck
                objectName: "magnetKeepStructure"
                text: qsTr("保持目录结构")
                checked: true
                font.pixelSize: theme.fontMeta
                // 用 checkedChanged 而不是 toggled：toggled 只在用户点击时发出，
                // 代码里改 checked（重开弹窗重置）不会同步到 keepStructure。
                onCheckedChanged: dialog.keepStructure = checked
            }
        }

        Item {
            Layout.fillWidth: true
            Layout.preferredHeight: dialog.listHeight

            Rectangle {
                anchors.fill: parent
                radius: theme.radiusControl
                color: theme.surfaceRaised
                border.width: 1
                border.color: theme.hairline
            }

            MagnetFileTree {
                id: fileTree
                objectName: "magnetFilesTree"
                anchors.fill: parent
                anchors.margins: 1
            }
        }
    }

    TaskFolderPickerDialog {
        id: folderPicker
        backend: dialog.backend
        onFolderChosen: function (folderId, pathText) {
            dialog.targetFolderId = folderId
            dialog.targetFolderPath = pathText
            dialog.pickerStartPath = folderPicker.selectedPath.slice()
        }
    }

    function openFor(folderId, folderPath, startPath) {
        if (!dialog.backend)
            return
        dialog.targetFolderId = folderId !== undefined && folderId !== null
                                ? String(folderId) : ""
        dialog.targetFolderPath = folderPath ? String(folderPath) : ""
        dialog.pickerStartPath = startPath ? startPath.slice() : []
        dialog.saveRequested = false
        dialog.keepStructure = true
        keepStructureCheck.checked = true
        fileTree.rows = dialog.backend.magnetFiles || []
        fileTree.load()
        dialog.open()
    }

    /// 提交后立即收起：进度与结果由底栏接管，用户不用守着弹窗。
    function save() {
        if (!dialog.canSave)
            return
        const indices = fileTree.selectedIndices()
        if (indices.length === 0)
            return
        dialog.backend.saveMagnetFiles(indices, dialog.targetFolderId, dialog.keepStructure)
        dialog.saveRequested = true
        dialog.close()
    }
}
