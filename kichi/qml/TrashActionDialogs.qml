pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 回收站的彻底删除确认。确认后等待后台结果，失败时保留原因。
Item {
    id: root

    property var backend: null
    property var targetIds: []
    property string targetName: ""
    property bool targetIsFolder: false
    readonly property int dialogWidth: 400

    KTheme { id: theme }

    KDialog {
        id: deleteDialog
        objectName: "trashDeleteDialog"
        title: qsTr("彻底删除")
        modal: true
        parent: Overlay.overlay
        anchors.centerIn: Overlay.overlay
        width: root.dialogWidth + leftPadding + rightPadding

        property string errorText: ""

        footer: DialogFooter {
            outerMargin: deleteDialog.padding
            okText: qsTr("彻底删除")
            okVariant: "danger"
            okEnabled: root.backend !== null && !root.backend.fileActionRunning
            cancelEnabled: okEnabled
            onOkClicked: root.submitDelete()
            onCancelClicked: deleteDialog.close()
        }

        onOpened: errorText = ""

        ColumnLayout {
            width: root.dialogWidth
            spacing: 8

            Label {
                objectName: "trashDeleteMessage"
                Layout.fillWidth: true
                wrapMode: Text.Wrap
                text: root.deleteMessage()
            }

            Label {
                Layout.fillWidth: true
                wrapMode: Text.Wrap
                font.pixelSize: theme.fontMeta
                color: theme.textMuted
                visible: root.backend && root.backend.fileActionRunning
                text: root.backend ? root.backend.statusMessage : ""
            }

            Label {
                objectName: "trashDeleteError"
                Layout.fillWidth: true
                wrapMode: Text.Wrap
                font.pixelSize: theme.fontMeta
                color: theme.danger
                visible: text.length > 0
                text: deleteDialog.errorText
            }
        }
    }

    function openDelete(entry, ids) {
        if (!entry || !root.backend)
            return
        targetName = entry.name
        targetIsFolder = entry.kind === "folder"
        targetIds = ids && ids.length > 0 ? ids.slice() : [entry.id]
        deleteDialog.open()
    }

    function deleteMessage() {
        const count = root.targetIds.length
        if (count > 1) {
            return qsTr("确定彻底删除选中的 %1 项吗？\n文件夹及其中内容会一并删除，此操作无法恢复。")
                    .arg(count)
        }
        return root.targetIsFolder
            ? qsTr("确定彻底删除文件夹「%1」吗？\n里面的所有文件和子文件夹都会一并删除，此操作无法恢复。")
                .arg(root.targetName)
            : qsTr("确定彻底删除「%1」吗？\n此操作无法恢复。").arg(root.targetName)
    }

    function submitDelete() {
        if (!root.backend || root.targetIds.length === 0)
            return
        deleteDialog.errorText = ""
        root.backend.deleteTrashEntries(root.targetIds)
    }

    Connections {
        target: root.backend

        function onFileActionDone(action, ok, message) {
            if (action === "trashDelete" && deleteDialog.visible) {
                if (ok)
                    deleteDialog.close()
                else
                    deleteDialog.errorText = message
            }
        }
    }
}
