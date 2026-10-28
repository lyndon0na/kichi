pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 文件页三个写操作弹窗：重命名 / 新建文件夹 / 批量移入回收站。
//
// 从 FilesPageMenu.qml 拆出来（那里已超行数软上限）：
// 菜单只管「点哪一项」，这里管「结果怎么显示」。
//
// 统一规则（docs/pitfalls.md 第 28 条）：不用 standardButtons。
// 点确定不关窗，结果回来才决定——失败原因留在弹窗里，成功才收起。
Item {
    id: root

    property var backend: null

    readonly property int dialogWidth: 380

    // 当前操作对象。多选时 target 是右键点到的那一条，targetIds 是全部。
    property var target: null
    property string targetId: ""
    property string targetName: ""
    property bool targetIsFolder: false
    property var targetIds: []

    // —— 重命名 ——
    Dialog {
        id: renameDialog
        objectName: "renameDialog"

        title: qsTr("重命名")
        modal: true
        // 位置一律按窗口算（docs/pitfalls.md 第 10 条）：挂在本组件自己的 Item 上
        // 会跟着宿主大小走，宿主是 0×0 时弹窗就跑到窗口左上角（2026-10-28 实测）。
        parent: Overlay.overlay
        anchors.centerIn: Overlay.overlay
        width: root.dialogWidth + leftPadding + rightPadding

        footer: DialogFooter {
            okEnabled: root.backend !== null && !root.backend.fileActionRunning
            onOkClicked: root.submitRename(nameField.text)
            onCancelClicked: renameDialog.close()
        }

        property string errorText: ""

        onOpened: {
            errorText = ""
            nameField.text = root.targetName
            // 选中主名、留出扩展名：改名时最常改的是前半段
            const dot = root.targetIsFolder ? -1 : root.targetName.lastIndexOf(".")
            if (dot > 0)
                nameField.select(0, dot)
            else
                nameField.selectAll()
            nameField.forceActiveFocus()
        }

        ColumnLayout {
            width: root.dialogWidth
            spacing: 8

            Label {
                Layout.fillWidth: true
                text: qsTr("原名称：%1").arg(root.targetName)
                color: palette.placeholderText
                elide: Text.ElideMiddle
            }

            TextField {
                id: nameField
                objectName: "renameNameField"
                Layout.fillWidth: true
                selectByMouse: true
                onAccepted: root.submitRename(text)
            }

            Label {
                objectName: "renameError"
                Layout.fillWidth: true
                wrapMode: Text.Wrap
                font.pixelSize: 12
                color: "#d9534f"
                visible: text.length > 0
                text: renameDialog.errorText
            }
        }
    }

    // —— 新建文件夹 ——
    Dialog {
        id: folderDialog
        objectName: "folderDialog"

        title: qsTr("新建文件夹")
        modal: true
        parent: Overlay.overlay
        anchors.centerIn: Overlay.overlay
        width: root.dialogWidth + leftPadding + rightPadding

        footer: DialogFooter {
            okEnabled: root.backend !== null && !root.backend.fileActionRunning
            onOkClicked: root.submitNewFolder(folderField.text)
            onCancelClicked: folderDialog.close()
        }

        property string errorText: ""

        onOpened: {
            errorText = ""
            folderField.text = ""
            folderField.forceActiveFocus()
        }

        ColumnLayout {
            width: root.dialogWidth
            spacing: 8

            TextField {
                id: folderField
                objectName: "folderNameField"
                Layout.fillWidth: true
                selectByMouse: true
                placeholderText: qsTr("文件夹名称")
                onAccepted: root.submitNewFolder(text)
            }

            Label {
                objectName: "folderError"
                Layout.fillWidth: true
                wrapMode: Text.Wrap
                font.pixelSize: 12
                color: "#d9534f"
                visible: text.length > 0
                text: folderDialog.errorText
            }
        }
    }

    // —— 移入回收站（可恢复，批量）——
    Dialog {
        id: trashDialog
        objectName: "trashDialog"

        title: qsTr("移至回收站")
        modal: true
        parent: Overlay.overlay
        anchors.centerIn: Overlay.overlay
        width: root.dialogWidth + leftPadding + rightPadding

        footer: DialogFooter {
            okText: qsTr("移入回收站")
            okEnabled: root.backend !== null && !root.backend.fileActionRunning
            onOkClicked: root.submitTrash()
            onCancelClicked: trashDialog.close()
        }

        property string errorText: ""

        onOpened: errorText = ""

        ColumnLayout {
            width: root.dialogWidth
            spacing: 8

            Label {
                objectName: "trashMessage"
                Layout.fillWidth: true
                wrapMode: Text.Wrap
                text: root.trashMessage()
            }

            Label {
                objectName: "trashError"
                Layout.fillWidth: true
                wrapMode: Text.Wrap
                font.pixelSize: 12
                color: "#d9534f"
                visible: text.length > 0
                text: trashDialog.errorText
            }
        }
    }

    // —— 入口 ——

    function openRename(entry) {
        if (!entry || !root.backend)
            return
        root.snapshot(entry, [entry.id])
        renameDialog.open()
    }

    function openNewFolder() {
        if (!root.backend)
            return
        folderDialog.open()
    }

    function openTrash(entry, ids) {
        if (!entry || !root.backend)
            return
        const list = ids && ids.length > 0 ? ids.slice() : [entry.id]
        root.snapshot(entry, list)
        trashDialog.open()
    }

    function snapshot(entry, ids) {
        target = entry
        targetId = entry.id
        targetName = entry.name
        targetIsFolder = entry.kind === "folder"
        targetIds = ids
    }

    function trashMessage() {
        const count = root.targetIds.length
        if (count > 1) {
            return qsTr("确定把选中的 %1 项移入回收站吗？\n条目可以在 PikPak 官方客户端里恢复；文件夹里的内容会一起移走。")
                .arg(count)
        }
        return root.targetIsFolder
            ? qsTr("确定把文件夹「%1」移入回收站吗？\n里面的所有文件和子文件夹都会一起移走。可以在 PikPak 官方客户端里恢复。")
                .arg(root.targetName)
            : qsTr("确定把「%1」移入回收站吗？\n可以在 PikPak 官方客户端里恢复。")
                .arg(root.targetName)
    }

    function submitRename(text) {
        const trimmed = String(text || "").trim()
        if (trimmed.length === 0 || !root.backend)
            return
        if (trimmed === root.targetName)
            return
        renameDialog.errorText = ""
        root.backend.renameEntry(root.targetId, trimmed)
    }

    function submitNewFolder(text) {
        const trimmed = String(text || "").trim()
        if (trimmed.length === 0 || !root.backend)
            return
        folderDialog.errorText = ""
        root.backend.createFolder(trimmed)
    }

    function submitTrash() {
        if (!root.backend || root.targetIds.length === 0)
            return
        trashDialog.errorText = ""
        root.backend.trashEntries(root.targetIds)
    }

    // 结果留在各自的弹窗里：失败原地显示原因，成功才收起
    Connections {
        target: root.backend

        function onFileActionDone(action, ok, message) {
            if (action === "rename" && renameDialog.visible) {
                if (ok) {
                    renameDialog.errorText = ""
                    renameDialog.close()
                } else {
                    renameDialog.errorText = message
                }
            } else if (action === "folder" && folderDialog.visible) {
                if (ok) {
                    folderDialog.close()
                } else {
                    folderDialog.errorText = message
                }
            } else if (action === "trash" && trashDialog.visible) {
                if (ok) {
                    trashDialog.close()
                } else {
                    trashDialog.errorText = message
                }
            }
        }
    }
}
