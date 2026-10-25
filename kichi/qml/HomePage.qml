import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 登录后的首页：S2 只做账号信息卡（文件页 / 任务页来自 S3、S4）
Item {
    id: page

    property var backend: null

    ColumnLayout {
        anchors.fill: parent
        spacing: 16

        Label {
            text: qsTr("已登录")
            font.pixelSize: 24
            font.bold: true
        }

        Frame {
            Layout.fillWidth: true

            ColumnLayout {
                anchors.fill: parent
                spacing: 8

                Label {
                    // 账号与容量由 Rust 侧同步后写在状态消息里
                    text: page.backend ? page.backend.statusMessage : ""
                    wrapMode: Text.Wrap
                    Layout.fillWidth: true
                }

                BusyIndicator {
                    running: page.backend ? page.backend.busy : false
                    visible: running
                    Layout.preferredWidth: 24
                    Layout.preferredHeight: 24
                }
            }
        }

        Label {
            text: qsTr("文件浏览与离线任务将在后续版本加入。")
            color: palette.placeholderText
        }

        Item { Layout.fillHeight: true }
    }
}
