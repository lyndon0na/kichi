pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 取消分享的异步确认框：结果回来前不关闭，失败原因留在弹窗里。
Item {
    id: root

    property var backend: null
    property var shareIds: []
    property string targetTitle: ""

    readonly property int shareCount: shareIds ? shareIds.length : 0
    readonly property bool running: backend ? backend.shareActionRunning : false

    signal revoked()

    property alias dialog: revokeDialog

    KTheme { id: theme }

    KDialog {
        id: revokeDialog
        objectName: "revokeSharesDialog"
        title: qsTr("取消分享")
        modal: true
        parent: Overlay.overlay
        anchors.centerIn: Overlay.overlay
        width: 400 + leftPadding + rightPadding
        spacing: theme.gapLg
        closePolicy: root.running ? Popup.NoAutoClose : Popup.CloseOnEscape

        footer: DialogFooter {
            outerMargin: revokeDialog.padding
            okText: qsTr("取消分享")
            okVariant: "danger"
            okEnabled: root.shareCount > 0 && !root.running
            cancelEnabled: !root.running
            onOkClicked: root.submit()
            onCancelClicked: revokeDialog.close()
        }

        property string errorText: ""

        onOpened: errorText = ""

        ColumnLayout {
            width: 400
            spacing: 8

            Label {
                objectName: "shareRevokeMessage"
                Layout.fillWidth: true
                wrapMode: Text.Wrap
                text: root.messageText()
                font.pixelSize: theme.fontBody
                color: theme.textPrimary
            }

            Label {
                objectName: "shareRevokeError"
                Layout.fillWidth: true
                wrapMode: Text.Wrap
                visible: text.length > 0
                text: revokeDialog.errorText
                font.pixelSize: theme.fontMeta
                color: theme.danger
            }
        }
    }

    Connections {
        target: root.backend

        function onShareActionDone(action, ok, message) {
            if (!revokeDialog.visible || action !== "revoke")
                return
            if (ok) {
                revokeDialog.close()
                root.revoked()
            } else {
                revokeDialog.errorText = message
            }
        }
    }

    function openFor(ids, title) {
        if (!backend || !ids || ids.length === 0)
            return
        shareIds = ids.slice()
        targetTitle = title || ""
        revokeDialog.errorText = ""
        revokeDialog.open()
    }

    function messageText() {
        if (root.shareCount > 1) {
            return qsTr("确定取消选中的 %1 个分享链接吗？\n链接会立即失效，网盘里的源文件不会被删除。")
                .arg(root.shareCount)
        }
        if (root.targetTitle.length > 0) {
            return qsTr("确定取消分享「%1」吗？\n链接会立即失效，网盘里的源文件不会被删除。")
                .arg(root.targetTitle)
        }
        return qsTr("确定取消这个分享链接吗？\n链接会立即失效，网盘里的源文件不会被删除。")
    }

    function submit() {
        if (!backend || root.shareCount === 0 || root.running)
            return
        revokeDialog.errorText = ""
        backend.revokeShares(root.shareIds.slice())
    }
}
