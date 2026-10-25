import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 登录页：账号密码 →（需要时）人机验证 → 登录成功
// 只调 backend 的 Q_INVOKABLE，不发任何网络请求
Item {
    id: page

    property var backend: null

    ColumnLayout {
        anchors.centerIn: parent
        width: Math.min(420, page.width)
        spacing: 12

        Label {
            text: "kichi"
            font.pixelSize: 36
            font.bold: true
            color: palette.text
            Layout.alignment: Qt.AlignHCenter
        }

        Label {
            text: page.backend && page.backend.authState === "expired"
                  ? qsTr("登录已失效，请重新登录")
                  : qsTr("使用 PikPak 账号登录")
            color: palette.placeholderText
            Layout.alignment: Qt.AlignHCenter
        }

        TextField {
            id: userField
            placeholderText: qsTr("邮箱或手机号")
            Layout.fillWidth: true
            enabled: page.backend && !page.backend.busy
            inputMethodHints: Qt.ImhNoPredictiveText | Qt.ImhNoAutoUppercase
        }

        TextField {
            id: passField
            placeholderText: qsTr("密码")
            echoMode: TextInput.Password
            Layout.fillWidth: true
            enabled: page.backend && !page.backend.busy
            onAccepted: page.submit()
        }

        // —— 人机验证分支 ——
        ColumnLayout {
            visible: page.backend && page.backend.captchaRequired
            Layout.fillWidth: true
            spacing: 8

            Frame {
                Layout.fillWidth: true

                ColumnLayout {
                    anchors.fill: parent
                    spacing: 8

                    Label {
                        text: qsTr("第 1 步：打开验证页面，按提示完成验证")
                        wrapMode: Text.Wrap
                        Layout.fillWidth: true
                    }
                    Button {
                        text: qsTr("打开验证页面")
                        onClicked: page.backend.openCaptchaPage()
                    }
                    Label {
                        text: qsTr("第 2 步：把验证页面上的验证令牌粘贴到下面，重新点登录")
                        wrapMode: Text.Wrap
                        Layout.fillWidth: true
                    }
                }
            }

            TextField {
                id: captchaField
                placeholderText: qsTr("验证令牌（captcha token）")
                Layout.fillWidth: true
                inputMethodHints: Qt.ImhNoPredictiveText | Qt.ImhNoAutoUppercase
            }
        }

        Button {
            text: page.backend && page.backend.busy ? qsTr("正在登录……") : qsTr("登录")
            Layout.fillWidth: true
            enabled: page.backend !== null && !page.backend.busy
                     && userField.text.length > 0 && passField.text.length > 0
            onClicked: page.submit()
        }

        Label {
            text: qsTr("登录令牌保存在系统钥匙串（KWallet）中，密码不会被保存。")
            color: palette.placeholderText
            font.pixelSize: 12
            wrapMode: Text.Wrap
            Layout.fillWidth: true
        }
    }

    function submit() {
        if (!backend || backend.busy) return
        backend.login(userField.text, passField.text, captchaField.text)
    }
}
