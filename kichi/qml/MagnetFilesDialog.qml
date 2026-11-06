pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 磁链解析结果：勾选要秒传的文件，并选择保存位置。
//
// 清单由 backend 的 resource/list 提供；这里只负责选择与进度显示。
// 保存成功后停在结果态，点「完成」才关闭并通知添加任务弹窗一起收起。
KDialog {
    id: dialog

    objectName: "magnetFilesDialog"
    property var backend: null
    /// 空串表示根目录；秒传必须落到一个具体父目录。
    property string targetFolderId: ""
    property string targetFolderPath: ""
    property var pickerStartPath: []
    /// select：勾选文件；result：保存结束后的结果页。
    property string stage: "select"
    property string resultText: ""
    property bool resultHasError: false
    /// 按磁链里的目录层级保存；关闭后所有选中文件直接平铺到目标目录。
    property bool keepStructure: true
    readonly property int checkedCount: fileTree.checkedCount
    readonly property int availableCount: fileTree.availableCount
    readonly property int totalFiles: fileTree.totalFiles

    signal saved(string message)

    readonly property int listHeight: 300
    readonly property bool saving: backend !== null && backend.magnetSaving === true
    readonly property bool allChecked: fileTree.allChecked
    readonly property bool canSave: backend !== null && stage === "select"
                                      && !saving && fileTree.checkedCount > 0
    readonly property string destinationText: targetFolderPath.length > 0
                                              ? targetFolderPath : qsTr("全部文件")
    readonly property string summaryText: backend !== null
                                          ? String(backend.magnetSummary || "") : ""
    readonly property string progressText: saving && backend !== null
                                           ? String(backend.magnetSaveStatus || "") : ""

    title: qsTr("选择要保存的文件")
    modal: true
    parent: Overlay.overlay
    anchors.centerIn: Overlay.overlay
    width: 560 + leftPadding + rightPadding
    closePolicy: saving ? Popup.NoAutoClose : Popup.CloseOnEscape

    onClosed: {
        const message = resultText
        const completed = stage === "result"
        stage = "select"
        resultText = ""
        resultHasError = false
        fileTree.clear()
        if (completed)
            saved(message)
    }

    footer: DialogFooter {
        outerMargin: dialog.padding
        statusText: dialog.stage === "result"
                    ? ""
                    : dialog.saving
                      ? dialog.progressText
                      : dialog.resultHasError ? dialog.resultText : ""
        statusColor: dialog.resultHasError ? theme.danger : theme.textMuted
        okText: dialog.stage === "result" ? qsTr("完成") : qsTr("保存选中")
        okEnabled: dialog.stage === "result" ? true : dialog.canSave
        cancelVisible: dialog.stage !== "result"
        cancelEnabled: !dialog.saving
        onOkClicked: {
            if (dialog.stage === "result")
                dialog.close()
            else
                dialog.save()
        }
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
            visible: dialog.stage === "select"
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
                enabled: !dialog.saving
                onClicked: folderPicker.openPicker(dialog.pickerStartPath)
            }
        }

        RowLayout {
            Layout.fillWidth: true
            visible: dialog.stage === "select"
            spacing: theme.gapSm

            KButton {
                objectName: "magnetSelectAll"
                text: dialog.allChecked ? qsTr("全不选") : qsTr("全选")
                variant: "ghost"
                implicitHeight: 24
                leftPadding: 8
                rightPadding: 8
                enabled: dialog.availableCount > 0 && !dialog.saving
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
                enabled: !dialog.saving
                font.pixelSize: theme.fontMeta
                // 用 checkedChanged 而不是 toggled：toggled 只在用户点击时发出，
                // 代码里改 checked（重开弹窗重置）不会同步到 keepStructure。
                onCheckedChanged: dialog.keepStructure = checked
            }
        }

        Item {
            Layout.fillWidth: true
            Layout.preferredHeight: dialog.listHeight
            visible: dialog.stage === "select"

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
                saving: dialog.saving
            }
        }

        // —— 结果态：占住同样的高度，关闭前不抖窗口 ——
        ColumnLayout {
            Layout.fillWidth: true
            Layout.preferredHeight: dialog.listHeight
            visible: dialog.stage === "result"
            spacing: theme.gapSm

            Item { Layout.fillHeight: true }

            NavIcon {
                Layout.alignment: Qt.AlignHCenter
                implicitWidth: 32
                implicitHeight: 32
                name: "download"
                color: theme.accent
            }

            Label {
                Layout.fillWidth: true
                horizontalAlignment: Text.AlignHCenter
                text: qsTr("已保存到云端")
                color: theme.textPrimary
                font.pixelSize: theme.fontSection
                font.weight: Font.DemiBold
            }

            Label {
                objectName: "magnetSaveResult"
                Layout.fillWidth: true
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.Wrap
                text: dialog.resultText
                color: theme.textSecondary
                font.pixelSize: theme.fontMeta
            }

            Item { Layout.fillHeight: true }
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

    Connections {
        target: dialog.backend
        ignoreUnknownSignals: true

        function onMagnetSaveFinished(savedCount, unavailableCount, failedCount, message) {
            if (!dialog.opened || dialog.stage !== "select")
                return
            dialog.resultText = String(message || "")
            if (savedCount > 0) {
                dialog.stage = "result"
                dialog.resultHasError = false
            } else {
                dialog.resultHasError = true
            }
        }
    }

    function openFor(folderId, folderPath, startPath) {
        if (!dialog.backend)
            return
        dialog.targetFolderId = folderId !== undefined && folderId !== null
                                ? String(folderId) : ""
        dialog.targetFolderPath = folderPath ? String(folderPath) : ""
        dialog.pickerStartPath = startPath ? startPath.slice() : []
        dialog.stage = "select"
        dialog.resultText = ""
        dialog.resultHasError = false
        dialog.keepStructure = true
        keepStructureCheck.checked = true
        fileTree.rows = dialog.backend.magnetFiles || []
        fileTree.load()
        dialog.open()
    }

    function save() {
        if (!dialog.canSave)
            return
        const indices = fileTree.selectedIndices()
        if (indices.length === 0)
            return
        dialog.resultText = ""
        dialog.resultHasError = false
        dialog.backend.saveMagnetFiles(indices, dialog.targetFolderId, dialog.keepStructure)
    }
}
