pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 移动 / 复制的目标文件夹选择器。
//
// 纯 QML 自绘（docs/pitfalls.md：不能构造 QWidget / 不能用原生目录选择框）。
// 只列文件夹；源文件夹在列表里置灰、不可进入——这样结构上就到不了
// 「把文件夹移进它自己的子目录」。目标等于源目录时确定键也置灰。
Dialog {
    id: picker
    objectName: "movePickerDialog"

    property var backend: null
    /// "move" | "copy"
    property string mode: "move"
    /// 要操作的条目 id（可多选）
    property var sourceIds: []
    /// 其中文件夹的 id：这些不能作为目标，也不能进入
    property var sourceFolderIds: []
    property string sourceName: ""
    property int sourceCount: 0
    /// 源目录 id（= 打开选择器时所在的目录）
    property string sourceParentId: ""
    /// 目标路径：根 → 当前目标，元素为 { id, name }
    property var path: []
    /// 操作失败原因（结果回来才填，弹窗不关）
    property string actionError: ""

    property int seenRevision: -1

    readonly property string targetId: path.length > 0 ? path[path.length - 1].id : ""
    readonly property string targetLabel: path.length > 0 ? path[path.length - 1].name
                                                          : qsTr("全部文件")
    readonly property bool canApply: backend !== null
        && !backend.moveTargetsLoading
        && !backend.fileActionRunning
        && sourceIds.length > 0
        && targetId !== sourceParentId
        && sourceIds.indexOf(targetId) < 0

    /// 弹窗内容宽度（子项统一撑满，别各自写死）
    readonly property int formWidth: 460

    title: mode === "move" ? qsTr("移动到…") : qsTr("复制到…")
    modal: true
    // 位置一律按窗口算（docs/pitfalls.md 第 10 条），不依赖宿主页面的大小
    parent: Overlay.overlay
    anchors.centerIn: Overlay.overlay
    width: formWidth + leftPadding + rightPadding
    closePolicy: backend && backend.fileActionRunning
                 ? Popup.NoAutoClose
                 : Popup.CloseOnEscape

    onClosed: {
        if (picker.backend)
            picker.backend.cancelMoveTargets()
    }

    // —— 打开入口 ——

    function openForMove(entry, ids, folderIds) {
        openFor("move", entry, ids, folderIds)
    }

    function openForCopy(entry, ids, folderIds) {
        openFor("copy", entry, ids, folderIds)
    }

    function openFor(mode, entry, ids, folderIds) {
        if (!backend || !entry)
            return
        picker.mode = mode
        picker.sourceIds = ids && ids.length > 0 ? ids.slice() : [entry.id]
        picker.sourceFolderIds = folderIds ? folderIds.slice() : []
        picker.sourceName = entry.name
        picker.sourceCount = picker.sourceIds.length
        picker.actionError = ""

        const raw = backend.currentPath() || []
        const full = []
        for (let i = 0; i < raw.length; i++)
            full.push({ id: raw[i].id, name: raw[i].name })
        if (full.length === 0)
            full.push({ id: "", name: qsTr("全部文件") })
        picker.sourceParentId = full[full.length - 1].id
        // 起点 = 当前目录的上一级：当前目录是源目录，本来就不能选
        const depth = Math.max(1, full.length - 1)
        picker.path = full.slice(0, depth)

        seenRevision = -1
        picker.open()
        picker.reload()
    }

    function reload() {
        actionError = ""
        foldersModel.clear()
        seenRevision = -1
        if (backend)
            backend.loadMoveTargets(targetId)
    }

    function goUp() {
        if (path.length <= 1)
            return
        path = path.slice(0, path.length - 1)
        reload()
    }

    function enterFolder(folder) {
        if (!folder || blocked(folder.id))
            return
        path = path.concat([{ id: folder.id, name: folder.name }])
        reload()
    }

    /// 源文件夹不能作为目标，也不能进入（否则可以绕到它的子目录）
    function blocked(id) {
        return sourceFolderIds.indexOf(id) >= 0
    }

    function apply() {
        if (!canApply)
            return
        actionError = ""
        if (mode === "move")
            backend.moveEntries(sourceIds, targetId, targetLabel)
        else
            backend.copyEntries(sourceIds, targetId, targetLabel)
    }

    function subject() {
        if (sourceCount > 1)
            return qsTr("选中的 %1 项").arg(sourceCount)
        return qsTr("「%1」").arg(sourceName)
    }

    // —— 文件夹列表（只在 revision 变化时重灌，避免无关 changed 重置滚动）——

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
            foldersModel.append({ folderId: list[i].id, folderName: list[i].name })
    }

    Connections {
        target: picker.backend

        function onChanged() {
            picker.syncFolders()
        }

        function onFileActionDone(action, ok, message) {
            if (!picker.opened || action !== picker.mode)
                return
            if (ok) {
                picker.actionError = ""
                picker.close()
            } else {
                picker.actionError = message
            }
        }
    }

    // —— 内容 ——

    ColumnLayout {
        width: picker.formWidth
        spacing: 8

        Label {
            Layout.fillWidth: true
            wrapMode: Text.Wrap
            text: picker.mode === "move"
                  ? qsTr("把%1移动到：").arg(picker.subject())
                  : qsTr("把%1复制到：").arg(picker.subject())
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: 6

            Button {
                objectName: "movePickerUp"
                text: qsTr("↑ 上一级")
                enabled: picker.backend !== null
                         && !picker.backend.moveTargetsLoading
                         && picker.path.length > 1
                onClicked: picker.goUp()
            }

            Label {
                objectName: "movePickerTarget"
                Layout.fillWidth: true
                elide: Text.ElideMiddle
                text: picker.targetLabel
                color: palette.link
            }

            BusyIndicator {
                visible: picker.backend !== null && picker.backend.moveTargetsLoading
                running: visible
                Layout.preferredWidth: 18
                Layout.preferredHeight: 18
            }
        }

        // 目标就是源目录：确定键已置灰，这里把原因说清楚
        Label {
            Layout.fillWidth: true
            wrapMode: Text.Wrap
            font.pixelSize: 12
            color: palette.placeholderText
            visible: picker.targetId === picker.sourceParentId
            text: qsTr("这里就是源目录；请进入其它文件夹或点「上一级」。")
        }

        ListView {
            id: folderList

            objectName: "movePickerList"
            Layout.fillWidth: true
            Layout.preferredHeight: 260
            clip: true
            model: foldersModel
            boundsBehavior: Flickable.StopAtBounds
            ScrollBar.vertical: KScrollBar {}

            delegate: ItemDelegate {
                required property string folderId
                required property string folderName

                width: folderList.width - 10
                text: picker.blocked(folderId)
                      ? folderName + qsTr("（源文件夹，不能选）")
                      : folderName
                enabled: !picker.blocked(folderId)
                onClicked: picker.enterFolder({ id: folderId, name: folderName })
            }
        }

        Label {
            objectName: "movePickerEmpty"
            Layout.fillWidth: true
            wrapMode: Text.Wrap
            color: palette.placeholderText
            visible: picker.backend !== null && !picker.backend.moveTargetsLoading
                     && foldersModel.count === 0 && picker.backend.moveTargetsError === ""
            text: qsTr("这个文件夹下没有子文件夹；可以直接操作到这里。")
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: picker.backend !== null && picker.backend.moveTargetsError !== ""

            Label {
                objectName: "movePickerListError"
                Layout.fillWidth: true
                wrapMode: Text.Wrap
                font.pixelSize: 12
                color: "#d9534f"
                text: picker.backend ? picker.backend.moveTargetsError : ""
            }

            Button {
                objectName: "movePickerRetry"
                text: qsTr("重试")
                onClicked: picker.reload()
            }
        }

        Label {
            objectName: "movePickerActionError"
            Layout.fillWidth: true
            wrapMode: Text.Wrap
            font.pixelSize: 12
            color: "#d9534f"
            visible: picker.actionError.length > 0
            text: picker.actionError
        }
    }

    footer: DialogFooter {
        okText: picker.mode === "move" ? qsTr("移动到这里") : qsTr("复制到这里")
        okEnabled: picker.canApply
        cancelEnabled: !(picker.backend && picker.backend.fileActionRunning)
        onOkClicked: picker.apply()
        onCancelClicked: picker.close()
    }
}
