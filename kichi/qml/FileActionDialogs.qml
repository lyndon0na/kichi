pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 文件页三个写操作弹窗：重命名 / 新建文件夹 / 删除。
//
// 从 FilesPageMenu.qml 拆出来（那里已超行数软上限）：
// 菜单只管「点哪一项」，这里管「结果怎么显示」。
//
// 统一规则：不用 standardButtons。
// 写入被 Rust 接受后立刻关窗；数据已经乐观更新，网络失败由底栏提示并回滚。
Item {
    id: root

    property var backend: null

    KTheme { id: theme }

    FileActionLocks {
        id: locks
        backend: root.backend
    }

    readonly property int dialogWidth: 380

    // 当前操作对象。多选时 target 是右键点到的那一条，targetIds 是全部。
    property var target: null
    property string targetId: ""
    property string targetName: ""
    property bool targetIsFolder: false
    property var targetIds: []
    property string folderTempId: ""

    // —— 重命名 ——
    KDialog {
        id: renameDialog
        objectName: "renameDialog"

        title: qsTr("重命名")
        modal: true
        // 位置一律按窗口算：挂在本组件自己的 Item 上
        // 会跟着宿主大小走，宿主是 0×0 时弹窗就跑到窗口左上角（2026-10-28 实测）。
        parent: Overlay.overlay
        anchors.centerIn: Overlay.overlay
        width: root.dialogWidth + leftPadding + rightPadding

        footer: DialogFooter {
            outerMargin: renameDialog.padding
            okEnabled: root.backend !== null && !root.backend.filesLoading
                       && !locks.any(root.targetIds)
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

            ElidedLabel {
                Layout.fillWidth: true
                text: qsTr("原名称：%1").arg(root.targetName)
                color: palette.placeholderText
                elide: Text.ElideMiddle
            }

            KTextField {
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
                color: theme.danger
                visible: text.length > 0
                text: renameDialog.errorText
            }
        }
    }

    // —— 新建文件夹 ——
    KDialog {
        id: folderDialog
        objectName: "folderDialog"

        title: qsTr("新建文件夹")
        modal: true
        parent: Overlay.overlay
        anchors.centerIn: Overlay.overlay
        width: root.dialogWidth + leftPadding + rightPadding

        footer: DialogFooter {
            outerMargin: folderDialog.padding
            okEnabled: root.backend !== null && !root.backend.filesLoading
            onOkClicked: root.submitNewFolder(folderField.text)
            onCancelClicked: folderDialog.close()
        }

        property string errorText: ""

        onOpened: {
            errorText = ""
            folderField.text = ""
            root.folderTempId = ""
            folderField.forceActiveFocus()
        }

        ColumnLayout {
            width: root.dialogWidth
            spacing: 8

            KTextField {
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
                color: theme.danger
                visible: text.length > 0
                text: folderDialog.errorText
            }
        }
    }

    // —— 删除（默认移入回收站；勾选后彻底删除）——
    KDialog {
        id: deleteDialog
        objectName: "deleteDialog"

        title: qsTr("删除")
        modal: true
        parent: Overlay.overlay
        anchors.centerIn: Overlay.overlay
        width: root.dialogWidth + leftPadding + rightPadding

        footer: DialogFooter {
            outerMargin: deleteDialog.padding
            okText: permanentCheck.checked ? qsTr("彻底删除") : qsTr("移到回收站")
            okVariant: permanentCheck.checked ? "danger" : "primary"
            okEnabled: root.backend !== null && !root.backend.filesLoading
                       && !locks.any(root.targetIds)
                       && (permanentCheck.checked
                           || root.backend.trashClearing !== true)
            onOkClicked: root.submitDelete()
            onCancelClicked: deleteDialog.close()
        }

        property string errorText: ""

        onOpened: {
            errorText = ""
            permanentCheck.checked = false
        }

        ColumnLayout {
            width: root.dialogWidth
            spacing: 8

            Label {
                objectName: "deleteMessage"
                Layout.fillWidth: true
                wrapMode: Text.Wrap
                color: permanentCheck.checked ? theme.danger : theme.textPrimary
                text: root.deleteMessage(permanentCheck.checked)
            }

            CheckBox {
                id: permanentCheck
                objectName: "deletePermanentCheck"
                text: qsTr("彻底删除")
            }

            Label {
                objectName: "deleteError"
                Layout.fillWidth: true
                wrapMode: Text.Wrap
                font.pixelSize: 12
                color: theme.danger
                visible: text.length > 0
                text: deleteDialog.errorText
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

    function openDelete(entry, ids) {
        if (!entry || !root.backend)
            return
        const list = ids && ids.length > 0 ? ids.slice() : [entry.id]
        root.snapshot(entry, list)
        deleteDialog.open()
    }

    function snapshot(entry, ids) {
        target = entry
        targetId = entry.id
        targetName = entry.name
        targetIsFolder = entry.kind === "folder"
        targetIds = ids
    }

    function deleteMessage(permanent) {
        const count = root.targetIds.length
        if (permanent) {
            if (count > 1)
                return qsTr("彻底删除选中的 %1 项？无法恢复。").arg(count)
            return root.targetIsFolder
                ? qsTr("彻底删除文件夹「%1」？内容一并删除，无法恢复。")
                    .arg(root.targetName)
                : qsTr("彻底删除「%1」？无法恢复。").arg(root.targetName)
        }
        if (count > 1)
            return qsTr("把选中的 %1 项移到回收站？").arg(count)
        return root.targetIsFolder
            ? qsTr("把文件夹「%1」移到回收站？内容会一并移走。")
                .arg(root.targetName)
            : qsTr("把「%1」移到回收站？").arg(root.targetName)
    }

    function submitRename(text) {
        const trimmed = String(text || "").trim()
        if (trimmed.length === 0 || !root.backend)
            return
        if (trimmed === root.targetName)
            return
        renameDialog.errorText = ""
        if (root.backend.renameEntry(root.targetId, trimmed) === true)
            renameDialog.close()
    }

    function submitNewFolder(text) {
        const trimmed = String(text || "").trim()
        if (trimmed.length === 0 || !root.backend)
            return
        folderDialog.errorText = ""
        if (root.backend.createFolder(trimmed) === true)
            folderDialog.close()
    }

    function submitDelete() {
        if (!root.backend || root.targetIds.length === 0)
            return
        deleteDialog.errorText = ""
        const accepted = permanentCheck.checked
                ? root.backend.deleteEntries(root.targetIds) === true
                : root.backend.trashEntries(root.targetIds) === true
        if (accepted)
            deleteDialog.close()
    }

    // 本地校验失败会同步返回 false；网络结果回来时窗口通常已经关闭。
    Connections {
        target: root.backend
        ignoreUnknownSignals: true

        function onFolderCreateStarted(id) {
            if (folderDialog.visible)
                root.folderTempId = id
        }

        function onFileActionDone(action, ok, message, ids) {
            if (action === "rename" && renameDialog.visible) {
                if (ids && ids.indexOf(root.targetId) < 0)
                    return
                if (ok) {
                    renameDialog.errorText = ""
                    renameDialog.close()
                } else {
                    renameDialog.errorText = message
                }
            } else if (action === "folder" && folderDialog.visible) {
                if (ids && ids.length > 0
                        && (root.folderTempId.length === 0
                            || ids.indexOf(root.folderTempId) < 0))
                    return
                if (ok) {
                    folderDialog.close()
                } else {
                    folderDialog.errorText = message
                }
            } else if ((action === "trash" || action === "delete")
                       && deleteDialog.visible) {
                if (ids && !ids.some(function (id) {
                        return root.targetIds.indexOf(id) >= 0
                    }))
                    return
                if (ok) {
                    deleteDialog.close()
                } else {
                    deleteDialog.errorText = message
                }
            }
        }
    }
}
