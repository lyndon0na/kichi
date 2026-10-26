import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 文件条目的右键菜单 + 重命名 / 新建文件夹 / 删除确认三个弹窗。
//
// 从 FilesPage.qml 拆出来：页面本体已接近行数软上限，而这些弹窗
// 都是「操作入口」，和列表渲染无关。
//
// 用法：放一个实例，调 openAt(point) 弹出右键菜单；
// 调 openRename(entry) / openNewFolder() 打开对应弹窗。
Item {
    id: root

    property var backend: null

    // 当前操作的条目（右键菜单打开时记下）
    property var target: null
    property string targetId: ""
    property string targetName: ""
    property bool targetIsFolder: false

    // —— 右键菜单 ——
    Menu {
        id: contextMenu

        // 宽度只在这里定，子项跟着撑满：
        // 每个 MenuItem 各自设 width 会比菜单容器还宽，高亮条就溢出去了
        width: 200

        // 在页面坐标系里 (point.x, point.y) 处弹出，超出边界时往回收
        function openAt(point) {
            var maxX = Math.max(0, root.width - width)
            var maxY = Math.max(0, root.height - height)
            popup(Math.max(0, Math.min(point.x, maxX)),
                  Math.max(0, Math.min(point.y, maxY)))
        }

        MenuItem {
            text: qsTr("重命名…")
            enabled: root.backend !== null && !root.backend.fileActionRunning
            onTriggered: root.openRename()
        }

        MenuItem {
            text: qsTr("移至回收站…")
            enabled: root.backend !== null && !root.backend.fileActionRunning
            onTriggered: trashDialog.open()
        }
    }

    // 三个弹窗统一的内容宽度。
    // 宽度只在这一处定：子项用 fillWidth 撑满，别再各自写死，
    // 否则漏写一处（如 TextField 的父布局没设宽度）就会溢出到弹窗外。
    readonly property int dialogWidth: 380

    // —— 重命名 ——
    Dialog {
        id: renameDialog

        title: qsTr("重命名")
        modal: true
        anchors.centerIn: parent
        standardButtons: Dialog.Ok | Dialog.Cancel
        width: root.dialogWidth + leftPadding + rightPadding

        property bool submitting: false
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

        onAccepted: root.submitRename(nameField.text)

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
                Layout.fillWidth: true
                selectByMouse: true
                onAccepted: root.submitRename(text)
            }

            Label {
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

        title: qsTr("新建文件夹")
        modal: true
        anchors.centerIn: parent
        standardButtons: Dialog.Ok | Dialog.Cancel
        width: root.dialogWidth + leftPadding + rightPadding

        property string errorText: ""

        onOpened: {
            errorText = ""
            folderField.text = ""
            folderField.forceActiveFocus()
        }
        onAccepted: {
            if (root.backend)
                root.backend.createFolder(folderField.text)
        }

        ColumnLayout {
            width: root.dialogWidth
            spacing: 8

            TextField {
                id: folderField
                Layout.fillWidth: true
                selectByMouse: true
                placeholderText: qsTr("文件夹名称")
                onAccepted: {
                    if (root.backend)
                        root.backend.createFolder(text)
                }
            }

            Label {
                Layout.fillWidth: true
                wrapMode: Text.Wrap
                font.pixelSize: 12
                color: "#d9534f"
                visible: text.length > 0
                text: folderDialog.errorText
            }
        }
    }

    // —— 删除确认（移到回收站，可恢复）——
    Dialog {
        id: trashDialog

        title: qsTr("移至回收站")
        modal: true
        anchors.centerIn: parent
        standardButtons: Dialog.Ok | Dialog.Cancel
        width: root.dialogWidth + leftPadding + rightPadding

        onAccepted: {
            if (root.backend && root.targetId.length > 0)
                root.backend.trashEntry(root.targetId)
        }

        Label {
            width: root.dialogWidth
            wrapMode: Text.Wrap
            text: root.targetIsFolder
                ? qsTr("确定把文件夹「%1」移入回收站吗？\n里面的所有文件和子文件夹都会一起移走。可以在 PikPak 官方客户端里恢复。")
                    .arg(root.targetName)
                : qsTr("确定把「%1」移入回收站吗？\n可以在 PikPak 官方客户端里恢复。")
                    .arg(root.targetName)
        }
    }

    // —— 入口 ——

    function openContext(entry, point) {
        if (!entry)
            return
        target = entry
        targetId = entry.id
        targetName = entry.name
        targetIsFolder = entry.kind === "folder"
        contextMenu.openAt(point)
    }

    function openRename() {
        if (!root.backend || root.targetId.length === 0)
            return
        renameDialog.open()
    }

    function openNewFolder() {
        folderDialog.open()
    }

    // 提交改名；名字没变就当作取消，不发请求
    function submitRename(text) {
        const trimmed = String(text || "").trim()
        if (trimmed.length === 0)
            return
        if (trimmed === root.targetName)
            return
        if (root.backend)
            root.backend.renameEntry(root.targetId, trimmed)
    }

    // 结果留在各自的对话框里：失败原地显示原因，成功才收起
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
            }
        }
    }
}
