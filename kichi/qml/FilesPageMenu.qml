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
    property bool targetPlayable: false
    // 这个条目是否已在下载队列里（打开菜单时问一次后端；见 openContext）
    property bool targetPending: false
    // 这个文件是否有可续传的断点（同一次打开菜单时问一次）
    property bool targetHasPartial: false
    // 右键处的位置（**窗口**坐标系）：清晰度菜单从这里弹出
    property point targetPoint: Qt.point(0, 0)

    // 菜单里点「播放」：页面把位置交给清晰度菜单，再让后端取档位
    signal playRequested(point anchor)

    // —— 右键菜单 ——
    Menu {
        id: contextMenu

        // 父项固定成窗口 overlay：Popup 的坐标是相对父项的，不固定就会跟着页面偏移
        // （与清晰度菜单同一条纪律，见 docs/pitfalls.md 第 10 条）
        parent: Overlay.overlay

        // 宽度只在这里定，子项跟着撑满：
        // 每个 MenuItem 各自设 width 会比菜单容器还宽，高亮条就溢出去了
        width: 200

        // 在窗口坐标系里 point 处弹出（左上角对准鼠标），和系统右键菜单一样。
        // 用 popup(Qt.point) 绝对定位，不用 x / y：那套坐标是相对本菜单父项的
        // （父项已固定为 overlay，见上）。超出窗口由 Menu 自己的 margins 夹回。
        function openAt(point) {
            popup(Qt.point(point.x, point.y))
        }

        // 只有视频/音频才有播放入口
        MenuItem {
            text: qsTr("播放（mpv）")
            visible: root.targetPlayable
            enabled: root.backend !== null && !root.backend.busy
                     && root.backend.playMenu.length === 0
            onTriggered: {
                // targetPoint 已经是窗口坐标，直接交给清晰度菜单
                root.playRequested(root.targetPoint)
                if (root.backend)
                    root.backend.requestPlay(root.targetId)
            }
        }

        // 下载到本地（文件直接下；文件夹递归下，保留层级）。
        // 同一条目已在队列里时不置灰：文案改成「去查看」，点了由后端跳转定位
        // （判重键是云端条目 id，不是文件名）。
        MenuItem {
            text: root.targetPending
                  ? qsTr("已在下载队列中，去查看")
                  : root.targetHasPartial ? qsTr("继续下载")
                  : root.targetIsFolder ? qsTr("下载整个文件夹到本地")
                                        : qsTr("下载到本地")
            enabled: root.backend !== null
            onTriggered: {
                if (root.backend)
                    root.backend.downloadEntry(root.targetId)
            }
        }

        MenuSeparator {}

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
        targetPlayable = entry.playable === true
        // 每次打开都重新问一次：条目可能刚下完（已释放）或刚被排上
        targetPending = root.backend !== null && root.backend.isDownloadPending(entry.id)
        // 文件夹没有「单文件断点」；只有文件才可能显示「继续下载」
        targetHasPartial = !targetIsFolder && root.backend !== null
                           && root.backend.hasPartialDownload(entry.id)
        targetPoint = point
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
