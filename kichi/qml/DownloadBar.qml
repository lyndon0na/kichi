import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 下载进度条：有下载在跑时出现（文件页底部）。
// 数据全部来自 backend 属性；取消按钮调 backend.cancelDownloads()。
Frame {
    id: bar

    property var backend: null

    visible: bar.backend !== null && bar.backend.downloadRunning
    padding: 8

    RowLayout {
        anchors.fill: parent
        spacing: 10

        BusyIndicator {
            running: bar.backend !== null && bar.backend.downloadRunning
            Layout.preferredWidth: 24
            Layout.preferredHeight: 24
        }

        Label {
            text: bar.backend ? bar.backend.downloadCurrent : ""
            elide: Text.ElideMiddle
            Layout.fillWidth: true
        }

        Label {
            text: bar.backend ? qsTr("剩 %1").arg(bar.backend.downloadRemaining) : ""
            color: palette.placeholderText
        }

        ToolButton {
            text: qsTr("取消")
            onClicked: {
                if (bar.backend)
                    bar.backend.cancelDownloads()
            }
        }
    }
}
