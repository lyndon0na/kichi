pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 从文件页创建分享：有效期、服务端生成密码、创建结果。
Item {
    id: root

    property var backend: null
    property var target: null
    property var targetIds: []

    readonly property int itemCount: targetIds ? targetIds.length : 0
    readonly property bool running: backend ? backend.shareActionRunning : false
    readonly property string itemLabel: {
        if (itemCount !== 1)
            return qsTr("选中的 %1 项").arg(itemCount)
        return target && target.name
            ? qsTr("「%1」").arg(target.name)
            : qsTr("选中的条目")
    }

    KTheme { id: theme }
    ClipboardHelper { id: clipboard }

    Timer {
        id: copyNoticeTimer
        interval: 1800
        onTriggered: shareDialog.copyNotice = ""
    }

    KDialog {
        id: shareDialog
        objectName: "shareCreateDialog"
        title: shareDialog.stage === "success" ? qsTr("分享已创建") : qsTr("创建分享")
        modal: true
        parent: Overlay.overlay
        anchors.centerIn: Overlay.overlay
        width: 420 + leftPadding + rightPadding
        closePolicy: root.running ? Popup.NoAutoClose : Popup.CloseOnEscape

        property string stage: "form"
        property string errorText: ""
        property string copyNotice: ""

        footer: DialogFooter {
            outerMargin: shareDialog.padding
            okText: shareDialog.stage === "success" ? qsTr("完成") : qsTr("创建链接")
            okEnabled: shareDialog.stage === "success" || !root.running
            cancelEnabled: !root.running
            cancelVisible: shareDialog.stage !== "success"
            onOkClicked: {
                if (shareDialog.stage === "success")
                    shareDialog.close()
                else
                    root.submit()
            }
            onCancelClicked: shareDialog.close()
        }

        onOpened: {
            stage = "form"
            errorText = ""
            copyNotice = ""
            expirationBox.currentIndex = 1
            customDays.text = ""
            passwordBox.checked = true
        }

        ColumnLayout {
            width: 420
            spacing: 12

            ColumnLayout {
                visible: shareDialog.stage === "form"
                Layout.fillWidth: true
                spacing: 10

                Label {
                    Layout.fillWidth: true
                    text: qsTr("为%1创建分享链接").arg(root.itemLabel)
                    wrapMode: Text.Wrap
                    font.pixelSize: theme.fontBody
                    font.weight: Font.DemiBold
                    color: theme.textPrimary
                }

                RowLayout {
                    Layout.fillWidth: true
                    spacing: theme.gapMd

                    Label {
                        text: qsTr("有效期")
                        font.pixelSize: theme.fontBody
                        color: theme.textPrimary
                        Layout.preferredWidth: 76
                    }

                    KComboBox {
                        id: expirationBox
                        objectName: "shareExpiration"
                        Layout.fillWidth: true
                        model: [
                            { "value": 1, "label": qsTr("1 天") },
                            { "value": 7, "label": qsTr("7 天") },
                            { "value": 30, "label": qsTr("30 天") },
                            { "value": "custom", "label": qsTr("自定义") },
                            { "value": -1, "label": qsTr("永久") }
                        ]
                        textRole: "label"
                        valueRole: "value"
                    }
                }

                RowLayout {
                    visible: expirationBox.currentIndex >= 0
                             && expirationBox.model[expirationBox.currentIndex].value === "custom"
                    Layout.fillWidth: true
                    spacing: 8

                    Item {
                        Layout.preferredWidth: 76
                    }

                    KTextField {
                        id: customDays
                        objectName: "shareCustomDays"
                        Layout.fillWidth: true
                        placeholderText: qsTr("输入天数")
                        selectByMouse: true
                        validator: IntValidator { bottom: 1; top: 99999 }
                        onAccepted: root.submit()
                    }

                    Label {
                        text: qsTr("天")
                        color: theme.textSecondary
                        font.pixelSize: theme.fontBody
                    }
                }

                CheckBox {
                    id: passwordBox
                    objectName: "sharePasswordRequired"
                    text: qsTr("需要访问密码")
                    checked: true
                }

                Label {
                    Layout.fillWidth: true
                    text: qsTr("访问密码由 PikPak 自动生成，创建后显示。链接会立即生效，任何拿到链接和密码的人都能访问。")
                    wrapMode: Text.Wrap
                    font.pixelSize: theme.fontMeta
                    color: theme.textMuted
                }

                RowLayout {
                    visible: root.running
                    spacing: theme.gapSm

                    BusyIndicator {
                        Layout.preferredWidth: 18
                        Layout.preferredHeight: 18
                        running: parent.visible
                    }

                    Label {
                        text: qsTr("正在创建分享链接…")
                        color: theme.textMuted
                        font.pixelSize: theme.fontMeta
                    }
                }

                Label {
                    objectName: "shareCreateError"
                    Layout.fillWidth: true
                    wrapMode: Text.Wrap
                    visible: text.length > 0
                    text: shareDialog.errorText
                    font.pixelSize: theme.fontMeta
                    color: theme.danger
                }
            }

            ColumnLayout {
                visible: shareDialog.stage === "success"
                Layout.fillWidth: true
                spacing: 10

                Label {
                    Layout.fillWidth: true
                    text: qsTr("%1的分享链接已创建，有效期：%2。")
                        .arg(root.itemLabel)
                        .arg(root.backend ? root.backend.shareCreatedExpiration : "")
                    wrapMode: Text.Wrap
                    font.pixelSize: theme.fontBody
                    color: theme.textPrimary
                }

                Label {
                    text: qsTr("分享链接")
                    font.pixelSize: theme.fontMeta
                    color: theme.textMuted
                }

                RowLayout {
                    Layout.fillWidth: true
                    spacing: 8

                    KTextField {
                        objectName: "shareCreatedUrl"
                        Layout.fillWidth: true
                        readOnly: true
                        selectByMouse: true
                        text: root.backend ? root.backend.shareCreatedUrl : ""
                    }

                    KButton {
                        objectName: "shareCopyLink"
                        text: qsTr("复制链接")
                        onClicked: {
                            if (clipboard.copyText(shareDialog.stage === "success"
                                                   ? root.backend.shareCreatedUrl : "")) {
                                shareDialog.copyNotice = qsTr("链接已复制")
                                copyNoticeTimer.restart()
                            }
                        }
                    }
                }

                ColumnLayout {
                    visible: root.backend && root.backend.shareCreatedPassCode.length > 0
                    Layout.fillWidth: true
                    spacing: 8

                    Label {
                        text: qsTr("访问密码")
                        font.pixelSize: theme.fontMeta
                        color: theme.textMuted
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 8

                        KTextField {
                            objectName: "shareCreatedPassword"
                            Layout.fillWidth: true
                            readOnly: true
                            selectByMouse: true
                            text: root.backend ? root.backend.shareCreatedPassCode : ""
                        }

                        KButton {
                            objectName: "shareCopyPassword"
                            text: qsTr("复制密码")
                            onClicked: {
                                if (clipboard.copyText(root.backend.shareCreatedPassCode)) {
                                    shareDialog.copyNotice = qsTr("密码已复制")
                                    copyNoticeTimer.restart()
                                }
                            }
                        }
                    }
                }

                Label {
                    Layout.fillWidth: true
                    text: qsTr("链接不会自动复制。请按需点击上方按钮。")
                    wrapMode: Text.Wrap
                    font.pixelSize: theme.fontMeta
                    color: theme.textMuted
                }

                Label {
                    Layout.fillWidth: true
                    visible: shareDialog.copyNotice.length > 0
                    text: shareDialog.copyNotice
                    font.pixelSize: theme.fontMeta
                    color: theme.success
                }
            }
        }
    }

    function openFor(entry, ids) {
        if (!backend || !ids || ids.length === 0)
            return
        target = entry
        targetIds = ids.slice()
        shareDialog.open()
    }

    function submit() {
        if (!backend || root.running || itemCount === 0)
            return
        const selected = expirationBox.model[expirationBox.currentIndex]
        let days = selected ? selected.value : 7
        if (days === "custom") {
            const value = Number(customDays.text)
            if (!Number.isInteger(value) || value <= 0) {
                shareDialog.errorText = qsTr("自定义有效期必须是大于 0 的整数天数")
                return
            }
            days = value
        }
        shareDialog.errorText = ""
        backend.createShare(root.targetIds, days, passwordBox.checked)
    }

    Connections {
        target: root.backend

        function onShareActionDone(action, ok, message) {
            if (!shareDialog.visible || action !== "create")
                return
            if (ok) {
                shareDialog.errorText = ""
                shareDialog.stage = "success"
            } else {
                shareDialog.errorText = message
            }
        }
    }
}
