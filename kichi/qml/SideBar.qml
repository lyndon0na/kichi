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

    readonly property bool compact: width < 190

    color: palette.button

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 14
        spacing: 8

        // 加载指示只在面包屑里（当前目录名后面），侧栏不再重复一个
        Label {
            text: "kichi"
            font.pixelSize: 24
            font.bold: true
            color: palette.windowText
        }

        Label {
            Layout.fillWidth: true
            text: qsTr("云盘与离线任务")
            elide: Text.ElideRight
            color: palette.placeholderText
            font.pixelSize: 12
        }

        Item { Layout.preferredHeight: 10 }

        Repeater {
            model: [
                { "label": qsTr("文件"), "short": qsTr("文件") },
                { "label": qsTr("离线任务"), "short": qsTr("任务") },
                { "label": qsTr("设置"), "short": qsTr("设置") }
            ]

            delegate: Button {
                id: navButton
                required property int index
                required property var modelData

                Layout.fillWidth: true
                Layout.preferredHeight: 40
                checked: bar.currentIndex === index
                highlighted: checked
                flat: true
                leftPadding: 13
                font.bold: checked
                onClicked: bar.pageRequested(index)

                contentItem: Label {
                    text: bar.compact ? modelData.short : modelData.label
                    font: navButton.font
                    color: navButton.highlighted ? palette.brightText : palette.windowText
                    horizontalAlignment: Text.AlignLeft
                    verticalAlignment: Text.AlignVCenter
                    elide: Text.ElideRight
                }
            }
        }

        Item { Layout.fillHeight: true }

        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: 1
            color: palette.mid
        }

        Label {
            Layout.fillWidth: true
            text: bar.backend && bar.backend.account.length > 0
                  ? bar.backend.account : qsTr("已登录")
            elide: Text.ElideMiddle
            color: palette.windowText
            font.pixelSize: 12
            ToolTip.visible: accountHover.hovered
            ToolTip.text: text

            HoverHandler {
                id: accountHover
            }
        }

        Label {
            Layout.fillWidth: true
            text: bar.backend ? bar.backend.quotaSummary : ""
            visible: text.length > 0
            elide: Text.ElideRight
            wrapMode: Text.Wrap
            maximumLineCount: 2
            color: palette.placeholderText
            font.pixelSize: 12
            ToolTip.visible: quotaHover.hovered
            ToolTip.text: text

            HoverHandler {
                id: quotaHover
            }
        }

        Button {
            id: signOutButton
            Layout.fillWidth: true
            text: qsTr("退出登录")
            flat: true
            leftPadding: 13
            onClicked: bar.signOutRequested()

            contentItem: Label {
                text: signOutButton.text
                font: signOutButton.font
                color: signOutButton.flat && !signOutButton.hovered
                       ? palette.windowText : palette.buttonText
                horizontalAlignment: Text.AlignLeft
                verticalAlignment: Text.AlignVCenter
            }
        }
    }
}
