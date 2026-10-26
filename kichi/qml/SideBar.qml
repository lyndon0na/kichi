import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 登录后的固定导航栏。账号与退出登录放在这里，内容区只负责当前页面。
Rectangle {
    id: bar

    property var backend: null
    property int currentIndex: 0

    signal pageRequested(int index)
    signal signOutRequested()

    /// 用户名：账号（邮箱）@ 前半段；没有账号时退回「已登录」
    function accountName() {
        if (!bar.backend)
            return ""
        var account = String(bar.backend.account)
        if (account.length === 0)
            return qsTr("已登录")
        var at = account.indexOf("@")
        return at > 0 ? account.substring(0, at) : account
    }

    /// 头像里的首字母
    function initial() {
        var name = bar.accountName()
        return name.length > 0 ? name.charAt(0).toUpperCase() : "K"
    }

    // 侧栏比内容区低半档：Breeze 下 Window 与 Button 是同一个颜色，
    // 直接铺满会看不出边界，用前景色薄薄压一层才有层次。
    readonly property color surfaceColor: Qt.tint(palette.window, Qt.rgba(0, 0, 0, 0.05))
    // 悬停 / 选中的底色取前景色与强调色加透明度，深浅主题都成立
    readonly property color hoverFill: Qt.rgba(palette.windowText.r, palette.windowText.g,
                                               palette.windowText.b, 0.07)
    readonly property color activeFill: Qt.rgba(palette.highlight.r, palette.highlight.g,
                                                palette.highlight.b, 0.26)
    readonly property color mutedColor: palette.placeholderText
    // 未选中的图标比正文淡一档。描边只有 ~1.6px，直接上强调色会在浅蓝底上糊成一片，
    // 选中态因此用「深色图标 + 强调色药丸底」来表达。
    readonly property color iconColor: Qt.rgba(palette.windowText.r, palette.windowText.g,
                                               palette.windowText.b, 0.62)

    color: surfaceColor

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 12
        spacing: 4

        // 加载指示只在面包屑里（当前目录名后面），侧栏不再重复一个
        Label {
            Layout.leftMargin: 6
            text: "kichi"
            font.pixelSize: 20
            font.bold: true
            color: palette.windowText
        }

        Label {
            Layout.fillWidth: true
            Layout.leftMargin: 6
            text: qsTr("云盘与离线任务")
            elide: Text.ElideRight
            color: bar.mutedColor
            font.pixelSize: 11
        }

        Item { Layout.preferredHeight: 8 }

        Repeater {
            model: [
                { "label": qsTr("我的文件"), "icon": "folder" },
                { "label": qsTr("离线任务"), "icon": "download" },
                { "label": qsTr("设置"), "icon": "settings" }
            ]

            delegate: Button {
                id: navButton
                required property int index
                required property var modelData

                Layout.fillWidth: true
                Layout.preferredHeight: 38
                padding: 0
                leftPadding: 10
                rightPadding: 10
                checkable: true
                checked: bar.currentIndex === index
                onClicked: bar.pageRequested(index)

                background: Rectangle {
                    radius: 7
                    color: navButton.checked ? bar.activeFill
                           : navButton.hovered ? bar.hoverFill : "transparent"

                    Behavior on color {
                        ColorAnimation { duration: 90 }
                    }
                }

                contentItem: RowLayout {
                    spacing: 10

                    NavIcon {
                        name: navButton.modelData.icon
                        color: navButton.checked ? palette.windowText : bar.iconColor
                        Layout.alignment: Qt.AlignVCenter
                    }

                    Label {
                        Layout.fillWidth: true
                        text: navButton.modelData.label
                        font.pixelSize: navButton.font.pixelSize
                        font.bold: navButton.checked
                        color: palette.windowText
                        elide: Text.ElideRight
                    }
                }
            }
        }

        Item { Layout.fillHeight: true }

        Rectangle {
            Layout.fillWidth: true
            Layout.bottomMargin: 4
            Layout.preferredHeight: 1
            color: Qt.rgba(palette.windowText.r, palette.windowText.g, palette.windowText.b, 0.10)
        }

        // —— 账号卡：头像 + 用户名 + 容量条 ——
        // 远端没有可用的头像图，用账号首字母做头像（同一账号颜色稳定）。
        RowLayout {
            Layout.fillWidth: true
            Layout.leftMargin: 4
            Layout.topMargin: 2
            spacing: 10

            Rectangle {
                Layout.alignment: Qt.AlignVCenter
                implicitWidth: 30
                implicitHeight: 30
                radius: 15
                color: bar.activeFill

                Label {
                    anchors.centerIn: parent
                    text: bar.initial()
                    color: palette.windowText
                    font.pixelSize: 14
                    font.bold: true
                }
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: 6

                Label {
                    Layout.fillWidth: true
                    text: bar.accountName()
                    color: palette.windowText
                    font.pixelSize: 12
                    font.bold: true
                    elide: Text.ElideRight
                    ToolTip.visible: accountHover.hovered
                    ToolTip.text: bar.backend ? bar.backend.account : ""

                    HoverHandler {
                        id: accountHover
                    }
                }

                Rectangle {
                    visible: bar.backend ? bar.backend.quotaVip : false
                    implicitWidth: vipLabel.implicitWidth + 10
                    implicitHeight: 15
                    radius: 7
                    color: bar.activeFill

                    Label {
                        id: vipLabel
                        anchors.centerIn: parent
                        text: qsTr("会员")
                        color: palette.windowText
                        font.pixelSize: 10
                    }
                }
            }
        }

        Label {
            Layout.fillWidth: true
            Layout.leftMargin: 4
            text: bar.backend && bar.backend.quotaLimit.length > 0
                  ? qsTr("已用 %1 / %2").arg(bar.backend.quotaUsed)
                                            .arg(bar.backend.quotaLimit)
                  : ""
            visible: text.length > 0
            elide: Text.ElideRight
            color: bar.mutedColor
            font.pixelSize: 11
            ToolTip.visible: quotaHover.hovered
            ToolTip.text: text

            HoverHandler {
                id: quotaHover
            }
        }

        // 容量条：quotaRatio < 0 表示不限量 / 未知，此时不画条
        Rectangle {
            Layout.fillWidth: true
            Layout.leftMargin: 4
            Layout.preferredHeight: 5
            visible: bar.backend ? bar.backend.quotaRatio >= 0 : false
            radius: height / 2
            color: bar.hoverFill

            Rectangle {
                width: parent.width * Math.max(bar.backend ? bar.backend.quotaRatio : 0, 0)
                height: parent.height
                radius: parent.radius
                color: palette.highlight
            }
        }

        Button {
            id: signOutButton
            Layout.fillWidth: true
            Layout.preferredHeight: 32
            padding: 0
            leftPadding: 10
            rightPadding: 10
            onClicked: bar.signOutRequested()

            background: Rectangle {
                radius: 7
                color: signOutButton.hovered ? bar.hoverFill : "transparent"

                Behavior on color {
                    ColorAnimation { duration: 90 }
                }
            }

            contentItem: RowLayout {
                spacing: 10

                NavIcon {
                    name: "logout"
                    color: signOutButton.hovered ? palette.windowText : bar.iconColor
                    Layout.alignment: Qt.AlignVCenter
                }

                Label {
                    Layout.fillWidth: true
                    text: qsTr("退出登录")
                    font: signOutButton.font
                    color: signOutButton.hovered ? palette.windowText : bar.mutedColor
                    elide: Text.ElideRight
                }
            }
        }
    }
}
