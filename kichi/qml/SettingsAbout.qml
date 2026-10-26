import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 设置页「关于」段：只读的存储位置信息。
// 从 SettingsPage 拆出来只为守 QML 行数提醒线，内容没有变化。
ColumnLayout {
    id: about

    property var backend: null

    spacing: 16

    Label {
        text: qsTr("关于")
        font.bold: true
    }

    GridLayout {
        Layout.fillWidth: true
        columns: 2
        columnSpacing: 12
        rowSpacing: 6

        Label {
            text: qsTr("令牌存储")
            color: palette.placeholderText
        }
        Label {
            text: about.backend ? about.backend.storageBackend : ""
            Layout.fillWidth: true
            wrapMode: Text.Wrap
        }

        Label {
            text: qsTr("设置文件")
            color: palette.placeholderText
        }
        Label {
            text: about.backend ? about.backend.settingsPath : ""
            Layout.fillWidth: true
            wrapMode: Text.WrapAnywhere
            font.family: "monospace"
            font.pixelSize: 12
        }
    }
}
