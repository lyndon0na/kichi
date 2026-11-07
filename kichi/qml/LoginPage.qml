import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 登录页：账号密码 →（需要时）人机验证 → 登录成功
// 只调 backend 的 Q_INVOKABLE，不发任何网络请求
//
// 视觉：居中的登录卡片，上方只有「应用名 + 英文副标题」两行，
// 副标题到卡片之间留一大段空白（比常规间距更大），把视线先落到名字再落到表单。
Item {
    id: page

    property var backend: null

    KTheme { id: theme }

    ColumnLayout {
        anchors.centerIn: parent
        width: Math.min(400, page.width - 48)
        spacing: 0

        Label {
            Layout.alignment: Qt.AlignHCenter
            text: "kichi"
            font.pixelSize: 30
            font.weight: Font.Bold
            color: theme.textPrimary
        }

        // 英文副标题：一句话说清「这是第三方客户端」，不再放 logo 和应用内口号
        Label {
            Layout.alignment: Qt.AlignHCenter
            text: "A third-party PikPak client"
            color: theme.textMuted
            font.pixelSize: theme.fontBody
        }

        // 副标题到表单之间单独留一大段空白（gapXl + gapLg = 40）
        Item { Layout.preferredHeight: theme.gapXl + theme.gapLg }

        // —— 登录卡片 ——
        Rectangle {
            Layout.fillWidth: true
            implicitHeight: form.implicitHeight + 2 * theme.gapLg
            radius: theme.radiusSurface
            color: theme.surfaceRaised
            border.width: 1
            border.color: theme.hairline

            ColumnLayout {
                id: form
                x: theme.gapLg
                y: theme.gapLg
                width: parent.width - 2 * theme.gapLg
                spacing: theme.gapMd

                // 登录失效时才出现的提示，平时不占位
                Label {
                    Layout.fillWidth: true
                    visible: page.backend && page.backend.authState === "expired"
                    text: qsTr("登录已失效，请重新登录")
                    wrapMode: Text.Wrap
                    color: theme.warning
                    font.pixelSize: theme.fontMeta
                }

                KTextField {
                    id: userField
                    placeholderText: qsTr("邮箱或手机号")
                    Layout.fillWidth: true
                    enabled: page.backend && !page.backend.loginRunning
                    inputMethodHints: Qt.ImhNoPredictiveText | Qt.ImhNoAutoUppercase
                }

                KTextField {
                    id: passField
                    placeholderText: qsTr("密码")
                    echoMode: TextInput.Password
                    Layout.fillWidth: true
                    enabled: page.backend && !page.backend.loginRunning
                    onAccepted: page.submit()
                }

                // —— 人机验证分支 ——
                ColumnLayout {
                    visible: page.backend && page.backend.captchaRequired
                    Layout.fillWidth: true
                    spacing: theme.gapSm

                    Rectangle {
                        Layout.fillWidth: true
                        implicitHeight: captchaSteps.implicitHeight + 2 * theme.gapMd
                        radius: theme.radiusCard
                        color: theme.accentGhost

                        ColumnLayout {
                            id: captchaSteps
                            x: theme.gapMd
                            y: theme.gapMd
                            width: parent.width - 2 * theme.gapMd
                            spacing: theme.gapSm

                            Label {
                                text: qsTr("第 1 步：打开验证页面，按提示完成验证")
                                wrapMode: Text.Wrap
                                Layout.fillWidth: true
                                font.pixelSize: theme.fontMeta
                                color: theme.textSecondary
                            }
                            KButton {
                                text: qsTr("打开验证页面")
                                variant: "secondary"
                                implicitHeight: 30
                                onClicked: page.backend.openCaptchaPage()
                            }
                            Label {
                                text: qsTr("第 2 步：把验证页面上的验证令牌粘贴到下面，重新点登录")
                                wrapMode: Text.Wrap
                                Layout.fillWidth: true
                                font.pixelSize: theme.fontMeta
                                color: theme.textSecondary
                            }
                        }
                    }

                    KTextField {
                        id: captchaField
                        placeholderText: qsTr("验证令牌（captcha token）")
                        Layout.fillWidth: true
                        inputMethodHints: Qt.ImhNoPredictiveText | Qt.ImhNoAutoUppercase
                    }
                }

                KButton {
                    text: page.backend && page.backend.loginRunning ? qsTr("正在登录……") : qsTr("登录")
                    variant: "primary"
                    implicitHeight: 38
                    Layout.fillWidth: true
                    Layout.topMargin: theme.gapXs
                    enabled: page.backend !== null && !page.backend.loginRunning
                             && userField.text.length > 0 && passField.text.length > 0
                    onClicked: page.submit()
                }
            }
        }

        Label {
            Layout.fillWidth: true
            Layout.topMargin: theme.gapMd
            text: qsTr("登录令牌保存在系统钥匙串中，密码不会被保存。")
            color: theme.textMuted
            font.pixelSize: theme.fontMeta
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.Wrap
        }
    }

    function submit() {
        if (!backend || backend.loginRunning) return
        backend.login(userField.text, passField.text, captchaField.text)
    }
}
