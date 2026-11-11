pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "../components"
import "../core"

// 打开别人的分享：粘贴链接 → 浏览分享目录 → 选中条目 → 选择保存位置转存。
//
// 解析、密码识别和分页都在 core / 会话层完成；这里只负责界面状态。
Item {
    id: root

    property var backend: null
    /// 当前层已选中的条目 id（进入 / 返回上一级时清空）
    property var selected: ({})

    readonly property bool busy: backend ? backend.shareOpenBusy : false
    readonly property bool ready: backend ? backend.shareOpenReady : false
    readonly property int selectedCount: selectedIds().length

    KTheme { id: theme }

    KDialog {
        id: dialog
        objectName: "shareOpenDialog"
        title: qsTr("打开分享链接")
        modal: true
        parent: Overlay.overlay
        anchors.centerIn: Overlay.overlay
        width: 560 + leftPadding + rightPadding
        closePolicy: (root.busy || dialog.saving) ? Popup.NoAutoClose : Popup.CloseOnEscape

        /// input | browse | done
        property string stage: "input"
        property string doneMessage: ""

        readonly property bool saving: root.backend ? root.backend.shareSaveRunning : false
        readonly property string browseError: root.backend ? root.backend.shareOpenError : ""

        onClosed: {
            if (root.backend)
                root.backend.closeShareOpen()
            saveDialog.close()
            root.selected = ({})
        }

        footer: DialogFooter {
            outerMargin: dialog.padding
            statusText: dialog.stage === "browse" && root.selectedCount > 0
                        ? qsTr("已选择 %1 项").arg(root.selectedCount) : ""
            statusColor: theme.textMuted
            okText: dialog.stage === "done" ? qsTr("完成")
                  : dialog.stage === "browse" ? qsTr("转存所选（%1）").arg(root.selectedCount)
                  : qsTr("打开")
            okEnabled: dialog.stage === "done"
                       ? true
                       : dialog.stage === "browse"
                         ? root.selectedCount > 0 && !root.busy && !dialog.saving
                         : linkField.text.trim().length > 0 && !root.busy
            cancelVisible: dialog.stage !== "done"
            cancelEnabled: !root.busy && !dialog.saving
            onOkClicked: {
                if (dialog.stage === "done")
                    dialog.close()
                else if (dialog.stage === "browse")
                    root.openSave()
                else
                    root.submit()
            }
            onCancelClicked: dialog.close()
        }

        ColumnLayout {
            width: 560
            spacing: theme.gapMd

            // —— 输入态 ——
            ColumnLayout {
                visible: dialog.stage === "input"
                Layout.fillWidth: true
                spacing: theme.gapSm

                Label {
                    Layout.fillWidth: true
                    wrapMode: Text.Wrap
                    text: qsTr("粘贴 PikPak 分享链接或整段分享文本。链接里的访问密码会自动识别，也可以单独填写。")
                    color: theme.textSecondary
                    font.pixelSize: theme.fontMeta
                }

                // 分享文本可能不止一行（标题 + 链接 + 访问密码），用多行框整段接住；
                // 内容超出高度时在框内滚动。真正拆链接和密码的是 core 里的解析器。
                ScrollView {
                    id: linkScroll
                    Layout.fillWidth: true
                    Layout.preferredHeight: 120
                    clip: true
                    // 车道补进 viewport，文字不会钻到滚动条底下。
                    rightPadding: linkBar.implicitWidth

                    // ScrollView 会替换掉默认滚动条的原定位绑定：必须自己挂
                    // parent + 三边锚点，滚动条才贴在右侧并占满整个框高。
                    ScrollBar.vertical: KScrollBar {
                        id: linkBar
                        objectName: "shareOpenLinkBar"
                        parent: linkScroll
                        anchors.top: parent.top
                        anchors.right: parent.right
                        anchors.bottom: parent.bottom
                        visible: size < 1
                    }

                    HoverHandler { id: linkHover }

                    background: Rectangle {
                        radius: theme.radiusControl
                        color: theme.surfaceRaised
                        border.width: linkField.activeFocus ? 2 : 1
                        border.color: linkField.activeFocus ? theme.accent
                                    : linkHover.hovered ? Qt.rgba(palette.text.r,
                                                                  palette.text.g,
                                                                  palette.text.b, 0.22)
                                    : theme.hairline
                    }

                    TextArea {
                        id: linkField
                        objectName: "shareOpenLinkInput"
                        width: linkScroll.availableWidth
                        placeholderText: qsTr("https://mypikpak.com/s/… 或整段分享文本")
                        wrapMode: TextEdit.Wrap
                        selectByMouse: true
                        font.pixelSize: theme.fontBody
                        color: theme.textPrimary
                        placeholderTextColor: theme.textMuted
                        leftPadding: 12
                        rightPadding: 12
                        topPadding: 8
                        bottomPadding: 8
                        background: null
                    }
                }

                KTextField {
                    id: codeField
                    objectName: "shareOpenPassCodeInput"
                    Layout.preferredWidth: 200
                    Layout.maximumWidth: 200
                    placeholderText: qsTr("访问密码（可选）")
                    selectByMouse: true
                    onAccepted: root.submit()
                }

                Label {
                    objectName: "shareOpenNeedsPassCode"
                    Layout.fillWidth: true
                    wrapMode: Text.Wrap
                    visible: root.backend !== null && root.backend.shareOpenNeedsPassCode
                    text: qsTr("这个分享需要访问密码，请填写后再打开。")
                    color: theme.warning
                    font.pixelSize: theme.fontMeta
                }

                RowLayout {
                    visible: root.busy
                    spacing: theme.gapSm

                    BusyIndicator {
                        Layout.preferredWidth: 18
                        Layout.preferredHeight: 18
                        running: parent.visible
                    }

                    Label {
                        text: qsTr("正在打开分享…")
                        color: theme.textMuted
                        font.pixelSize: theme.fontMeta
                    }
                }

                Label {
                    objectName: "shareOpenInputError"
                    Layout.fillWidth: true
                    wrapMode: Text.Wrap
                    visible: text.length > 0
                    text: dialog.browseError
                    color: theme.danger
                    font.pixelSize: theme.fontMeta
                }
            }

            // —— 浏览态 ——
            ShareOpenBrowsePane {
                visible: dialog.stage === "browse"
                Layout.fillWidth: true
                backend: root.backend
                selected: root.selected
                onToggleRequested: function (id) { root.toggle(id) }
                onEnterRequested: function (id, name) { root.enterFolder(id, name) }
                onGoUpRequested: root.goUp()
                onToggleAllRequested: root.toggleAll()
            }

            // —— 完成态 ——
            ColumnLayout {
                visible: dialog.stage === "done"
                Layout.fillWidth: true
                spacing: theme.gapSm

                Label {
                    objectName: "shareOpenDoneMessage"
                    Layout.fillWidth: true
                    wrapMode: Text.Wrap
                    text: dialog.doneMessage
                    color: theme.success
                    font.pixelSize: theme.fontBody
                    font.weight: Font.DemiBold
                }

                Label {
                    Layout.fillWidth: true
                    wrapMode: Text.Wrap
                    text: qsTr("可以在文件页的目标文件夹里找到转存的内容。")
                    color: theme.textMuted
                    font.pixelSize: theme.fontMeta
                }
            }
        }
    }

    ShareSaveDialog {
        id: saveDialog
        backend: root.backend
    }

    Connections {
        target: root.backend

        function onChanged() {
            if (!dialog.visible)
                return
            if (dialog.stage === "input" && root.ready)
                dialog.stage = "browse"
            if (root.backend && root.backend.shareOpenNeedsPassCode)
                codeField.forceActiveFocus()
        }

        function onShareRestoreDone(ok, message) {
            if (!dialog.visible || dialog.stage !== "browse")
                return
            if (ok) {
                saveDialog.close()
                dialog.doneMessage = message
                dialog.stage = "done"
            } else if (saveDialog.visible) {
                saveDialog.errorText = message
            }
        }
    }

    function openDialog() {
        if (!backend)
            return
        linkField.text = ""
        codeField.text = ""
        dialog.doneMessage = ""
        dialog.stage = "input"
        root.selected = ({})
        backend.closeShareOpen()
        dialog.open()
        linkField.forceActiveFocus()
    }

    function submit() {
        if (!backend || root.busy)
            return
        const text = linkField.text.trim()
        if (text.length === 0)
            return
        backend.openShareLink(text, codeField.text)
    }

    function openSave() {
        const ids = selectedIds()
        if (ids.length === 0)
            return
        saveDialog.openFor(ids)
    }

    function toggle(id) {
        const next = Object.assign({}, root.selected)
        if (next[id] === true)
            delete next[id]
        else if (id)
            next[id] = true
        root.selected = next
    }

    function toggleAll() {
        const rows = root.backend && root.backend.shareOpenRows
                     ? root.backend.shareOpenRows : []
        const allSelected = rows.length > 0 && selectedIds().length === rows.length
        const next = ({})
        if (!allSelected) {
            for (let i = 0; i < rows.length; i++) {
                if (rows[i].id)
                    next[rows[i].id] = true
            }
        }
        root.selected = next
    }

    function selectedIds() {
        const rows = root.backend && root.backend.shareOpenRows
                     ? root.backend.shareOpenRows : []
        const ids = []
        for (let i = 0; i < rows.length; i++) {
            if (root.selected[rows[i].id] === true)
                ids.push(rows[i].id)
        }
        return ids
    }

    function enterFolder(id, name) {
        if (!backend || !id)
            return
        root.selected = ({})
        backend.openShareFolder(id, name)
    }

    function goUp() {
        if (!backend)
            return
        root.selected = ({})
        backend.shareOpenGoUp()
    }
}
