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
                    text: qsTr("账号：%1").arg(
                              page.backend && page.backend.account.length > 0
                              ? page.backend.account : qsTr("同步中……"))
                    wrapMode: Text.Wrap
                    Layout.fillWidth: true
                }

                Label {
                    text: qsTr("空间：%1").arg(
                              page.backend && page.backend.quotaSummary.length > 0
                              ? page.backend.quotaSummary : qsTr("同步中……"))
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
