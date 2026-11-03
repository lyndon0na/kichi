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
    property var targetNames: []

    readonly property int itemCount: targetIds ? targetIds.length : 0
    readonly property string shareTitle: {
        const names = []
        for (let i = 0; i < (root.targetNames ? root.targetNames.length : 0); i++) {
            const name = String(root.targetNames[i] || "").trim()
            if (name.length > 0)
                names.push(name)
        }
        if (names.length === root.itemCount && names.length > 0)
            return names.join("、")
        if (root.itemCount === 1 && root.target && root.target.name)
            return String(root.target.name).trim()
        return root.itemCount > 1 ? qsTr("选中的 %1 项").arg(root.itemCount) : ""
    }
    readonly property bool running: backend ? backend.shareActionRunning : false
    readonly property string itemLabel: {
        if (itemCount !== 1)
            return qsTr("选中的 %1 项").arg(itemCount)
        return target && target.name
            ? qsTr("「%1」").arg(target.name)
            : qsTr("选中的条目")
    }
    readonly property string shareCreatedText: backend
        ? clipboard.shareText(backend.shareCreatedUrl,
                              backend.shareCreatedPassCode,
                              root.shareTitle)
        : ""
    readonly property bool shareHasPassword: backend
        && String(backend.shareCreatedPassCode || "").trim().length > 0

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
        spacing: theme.gapLg
        closePolicy: root.running ? Popup.NoAutoClose : Popup.CloseOnEscape

        property string stage: "form"
        property string errorText: ""
        property string copyNotice: ""

        footer: DialogFooter {
            outerMargin: shareDialog.padding
            statusText: shareDialog.stage !== "success"
                        ? ""
                        : shareDialog.copyNotice.length > 0
                          ? shareDialog.copyNotice
                          : root.shareHasPassword
                            ? qsTr("点击复制会同时带上访问密码。")
                            : qsTr("点击复制链接。")
            statusColor: shareDialog.copyNotice.length > 0
                         ? theme.success : theme.textMuted
            okObjectName: shareDialog.stage === "success"
                          ? "shareCopyLink" : "dialogFooterOk"
            okText: shareDialog.stage === "success" ? qsTr("复制链接") : qsTr("创建链接")
            okEnabled: shareDialog.stage === "success"
                       ? root.shareCreatedText.length > 0
                       : !root.running
            cancelObjectName: shareDialog.stage === "success"
                              ? "shareDoneButton" : "dialogFooterCancel"
            cancelText: shareDialog.stage === "success" ? qsTr("完成") : qsTr("取消")
            cancelEnabled: !root.running
            onOkClicked: {
                if (shareDialog.stage === "success")
                    root.copyCreatedShare()
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

            ShareResultPane {
                visible: shareDialog.stage === "success"
                Layout.fillWidth: true
                itemLabel: root.itemLabel
                expirationText: root.backend ? root.backend.shareCreatedExpiration : ""
                shareText: root.shareCreatedText
                hasPassword: root.shareHasPassword
            }
        }
    }

    function openFor(entry, ids, names) {
        if (!backend || !ids || ids.length === 0)
            return
        target = entry
        targetIds = ids.slice()
        targetNames = names && names.length > 0 ? names.slice() : []
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

    function copyCreatedShare() {
        if (!clipboard.copyText(root.shareCreatedText))
            return
        shareDialog.copyNotice = root.shareHasPassword
                ? qsTr("链接和密码已复制")
                : qsTr("链接已复制")
        copyNoticeTimer.restart()
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
