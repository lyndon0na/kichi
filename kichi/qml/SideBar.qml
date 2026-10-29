import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 登录后的固定导航栏。账号信息放在底部，内容区只负责当前页面。
// 退出登录在设置页「账号」一节（低频、不可逆，别放回侧栏）。
//
// 视觉：侧栏底色比内容区低半档，导航项是圆角卡片；选中态用「淡强调色底 +
// 3px 竖条 + 强调色图标」，不铺满饱和色块（与文件列表的选中语言一致）。
Rectangle {
    id: bar

    property var backend: null
    property int currentIndex: 0

    signal pageRequested(int index)

    KTheme { id: theme }

    color: theme.surfaceSheet

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: theme.gapMd
        spacing: theme.gapXs

        Repeater {
            model: [
                { "label": qsTr("我的文件"), "icon": "folder" },
                { "label": qsTr("分享"), "icon": "share" },
                { "label": qsTr("传输任务"), "icon": "transfer" },
                { "label": qsTr("离线任务"), "icon": "download" },
                { "label": qsTr("回收站"), "icon": "trash" },
                { "label": qsTr("设置"), "icon": "settings" }
            ]

            delegate: Button {
                id: navButton
                required property int index
                required property var modelData

                Layout.fillWidth: true
                Layout.preferredHeight: 40
                padding: 0
                leftPadding: 12
                rightPadding: 12
                checkable: true
                checked: bar.currentIndex === index
                onClicked: bar.pageRequested(index)

                background: Rectangle {
                    radius: theme.radiusCard
                    color: navButton.checked ? theme.accentSoft
                         : navButton.hovered ? theme.surfaceHover : theme.surfaceClear


                    // 选中竖条：和文件列表的选中语言一致
                    Rectangle {
                        visible: navButton.checked
                        width: 3
                        height: parent.height - 16
                        radius: 1.5
                        anchors.left: parent.left
                        anchors.leftMargin: 1
                        anchors.verticalCenter: parent.verticalCenter
                        color: theme.accent
                    }
                }

                contentItem: RowLayout {
                    spacing: 10

                    NavIcon {
                        implicitWidth: 18
                        implicitHeight: 18
                        name: navButton.modelData.icon
                        color: navButton.checked ? theme.accent : theme.textMuted
                        Layout.alignment: Qt.AlignVCenter
                    }

                    Label {
                        Layout.fillWidth: true
                        text: navButton.modelData.label
                        font.pixelSize: theme.fontBody
                        font.weight: navButton.checked ? Font.DemiBold : Font.Normal
                        color: navButton.checked ? theme.textPrimary : theme.textSecondary
                        elide: Text.ElideRight
                    }

                    // 传输任务：进行中条数徽标（0 时隐藏）。
                    Rectangle {
                        readonly property int count: navButton.modelData.icon === "transfer"
                                                    && bar.backend ? bar.backend.downloadActive : 0

                        Layout.alignment: Qt.AlignVCenter
                        visible: count > 0
                        implicitWidth: badgeLabel.implicitWidth + 12
                        implicitHeight: 16
                        radius: theme.radiusSmall
                        color: theme.accent

                        Label {
                            id: badgeLabel
                            anchors.centerIn: parent
                            text: parent.count > 99 ? "99+" : String(parent.count)
                            color: theme.onAccent
                            font.pixelSize: theme.fontCaption
                            font.weight: Font.DemiBold
                        }
                    }
                }
            }
        }

        Item { Layout.fillHeight: true }

        AccountCard {
            Layout.fillWidth: true
            Layout.bottomMargin: theme.gapXs
            account: bar.backend ? String(bar.backend.account) : ""
            quotaUsed: bar.backend ? bar.backend.quotaUsed : ""
            quotaLimit: bar.backend ? bar.backend.quotaLimit : ""
            quotaRatio: bar.backend ? bar.backend.quotaRatio : -1
            vip: bar.backend ? bar.backend.quotaVip : false
        }
    }
}
