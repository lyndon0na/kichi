pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "../components"
import "../files"

// 移动 / 复制的目标文件夹选择器。
//
// 纯 QML 自绘：不能构造 QWidget / 不能用原生目录选择框。
// 只列文件夹；源文件夹在列表里置灰、不可进入——这样结构上就到不了
// 「把文件夹移进它自己的子目录」。目标等于源目录时确定键也置灰。
KDialog {
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
    property alias path: nav.path
    /// 同步校验失败原因；网络失败在窗口关闭后由底栏提示
    property string actionError: ""

    FileActionLocks {
        id: locks
        backend: picker.backend
    }

    FolderPickerNavigation {
        id: nav
        backend: picker.backend
        fileList: folderList
        canEnter: function (id) {
            return !picker.blocked(id)
        }
    }

    // 主题令牌来自 KDialog 的 `theme` 属性（见 KDialog.qml 的说明：
    // 不能再写一个 `KTheme { }` 子项，否则弹窗正文会塌成 0 高）。

    readonly property string targetId: nav.targetId
    readonly property string targetLabel: path.length > 0 ? path[path.length - 1].name
                                                          : qsTr("全部文件")
    readonly property bool canApply: backend !== null
        && !locks.any(sourceIds)
        && sourceIds.length > 0
        && targetId !== sourceParentId
        && sourceIds.indexOf(targetId) < 0

    /// 弹窗内容宽度（子项统一撑满，别各自写死）
    readonly property int formWidth: 460
    /// 文件夹列表高度固定：提示出现 / 消失时弹窗和列表都不动
    readonly property int listHeight: 224
    readonly property int hintHeight: 36
    readonly property bool hasListError: backend !== null
                                          && backend.moveTargetsError !== ""

    title: mode === "move" ? qsTr("移动到…") : qsTr("复制到…")
    modal: true
    // 位置一律按窗口算，不依赖宿主页面的大小
    parent: Overlay.overlay
    anchors.centerIn: Overlay.overlay
    width: formWidth + leftPadding + rightPadding
    closePolicy: Popup.CloseOnEscape

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

        nav.reset()
        picker.open()
        picker.reload()
    }

    function reload() {
        actionError = ""
        nav.reload()
    }

    /// 手动刷新：跳过本地快照强制联网，刷新后留在原位置
    function refresh() {
        if (!backend || backend.moveTargetsLoading)
            return
        actionError = ""
        nav.refresh()
    }

    function goUp() {
        if (path.length <= 1)
            return
        actionError = ""
        nav.goUp()
    }

    function enterFolder(folder) {
        if (!folder || blocked(folder.id))
            return
        actionError = ""
        nav.enterFolder(folder)
    }

    /// 源文件夹不能作为目标，也不能进入（否则可以绕到它的子目录）
    function blocked(id) {
        return sourceFolderIds.indexOf(id) >= 0
    }

    function apply() {
        if (!canApply)
            return
        actionError = ""
        const accepted = mode === "move"
                       ? backend.moveEntries(sourceIds, targetId, targetLabel)
                       : backend.copyEntries(sourceIds, targetId, targetLabel)
        if (accepted === true)
            picker.close()
    }

    function subject() {
        if (sourceCount > 1)
            return qsTr("选中的 %1 项").arg(sourceCount)
        return qsTr("「%1」").arg(sourceName)
    }

    // —— 文件夹列表（只在 revision 变化时重灌，避免无关 changed 重置滚动）——

    Connections {
        target: picker.backend

        function onFileActionDone(action, ok, message, ids) {
            if (!picker.opened || action !== picker.mode)
                return
            if (!ids || !ids.some(function (id) {
                    return picker.sourceIds.indexOf(id) >= 0
                }))
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
        spacing: picker.theme.gapSm

        Label {
            Layout.fillWidth: true
            wrapMode: Text.Wrap
            maximumLineCount: 1
            elide: Text.ElideRight
            text: picker.mode === "move"
                  ? qsTr("把%1移动到：").arg(picker.subject())
                  : qsTr("把%1复制到：").arg(picker.subject())
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: picker.theme.gapXs

            KButton {
                objectName: "movePickerUp"
                text: qsTr("↑ 上一级")
                variant: "secondary"
                implicitHeight: 30
                enabled: picker.backend !== null && picker.path.length > 1
                onClicked: picker.goUp()
            }

            ElidedLabel {
                objectName: "movePickerTarget"
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
                objectName: "movePickerRefresh"
                text: qsTr("刷新")
                variant: "secondary"
                implicitHeight: 30
                enabled: picker.backend !== null && !picker.backend.moveTargetsLoading
                onClicked: picker.refresh()
            }
        }

        FolderPickerListView {
            id: folderList

            objectName: "movePickerList"
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

                width: folderList.width - 10
                text: picker.blocked(folderId)
                      ? folderName + qsTr("（源文件夹，不能选）")
                      : folderName
                enabled: !picker.blocked(folderId)
                onClicked: picker.enterFolder({ id: folderId, name: folderName })
            }
        }

        // 固定高度的提示条：源目录提示 / 空目录 / 错误 / 操作失败都放这里。
        // 状态切换时弹窗和列表高度不变——列表一变高，Qt 会异步恢复旧滚动位置。
        // 优先级：操作失败 > 列表错误 > 源目录提示 > 空目录。
        Item {
            Layout.fillWidth: true
            Layout.preferredHeight: picker.hintHeight

            Label {
                objectName: "movePickerActionError"
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                wrapMode: Text.Wrap
                maximumLineCount: 2
                elide: Text.ElideRight
                font.pixelSize: picker.theme.fontMeta
                color: picker.theme.danger
                visible: picker.actionError.length > 0
                text: picker.actionError
            }

            Label {
                objectName: "movePickerListError"
                anchors.left: parent.left
                anchors.right: retry.visible ? retry.left : parent.right
                anchors.rightMargin: retry.visible ? picker.theme.gapSm : 0
                anchors.verticalCenter: parent.verticalCenter
                wrapMode: Text.Wrap
                maximumLineCount: 2
                elide: Text.ElideRight
                font.pixelSize: picker.theme.fontMeta
                color: picker.theme.danger
                visible: picker.actionError.length === 0 && picker.hasListError
                text: picker.backend ? picker.backend.moveTargetsError : ""
            }

            KButton {
                id: retry
                objectName: "movePickerRetry"
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                text: qsTr("重试")
                variant: "secondary"
                implicitHeight: 28
                visible: picker.actionError.length === 0 && picker.hasListError
                onClicked: picker.refresh()
            }

            // 目标就是源目录：确定键已置灰，这里把原因说清楚
            Label {
                objectName: "movePickerNote"
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                wrapMode: Text.Wrap
                maximumLineCount: 2
                elide: Text.ElideRight
                font.pixelSize: picker.theme.fontMeta
                color: picker.theme.textMuted
                visible: picker.actionError.length === 0 && !picker.hasListError
                         && picker.targetId === picker.sourceParentId
                text: qsTr("这里就是源目录；请进入其它文件夹或点「上一级」。")
            }

            Label {
                objectName: "movePickerEmpty"
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                wrapMode: Text.Wrap
                maximumLineCount: 2
                elide: Text.ElideRight
                font.pixelSize: picker.theme.fontMeta
                color: picker.theme.textMuted
                visible: picker.actionError.length === 0 && !picker.hasListError
                         && picker.targetId !== picker.sourceParentId
                         && picker.backend !== null && !picker.backend.moveTargetsLoading
                         && nav.foldersModel.count === 0
                text: qsTr("这个文件夹下没有子文件夹；可以直接操作到这里。")
            }
        }
    }

    footer: DialogFooter {
        outerMargin: picker.padding
        okText: picker.mode === "move" ? qsTr("移动到这里") : qsTr("复制到这里")
        okEnabled: picker.canApply
        cancelEnabled: true
        onOkClicked: picker.apply()
        onCancelClicked: picker.close()
    }
}
