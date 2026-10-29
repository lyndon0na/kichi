import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 设置页「关于」段：只读的本机存储位置。
ColumnLayout {
    id: about

    property var backend: null

    readonly property KTheme theme: KTheme { }

    spacing: 0

    KSettingsGroup {
        title: qsTr("关于 kichi")
        Layout.fillWidth: true

        KSettingsRow {
            title: qsTr("设置文件")

            Label {
                id: settingsPath
                objectName: "settingsPath"
                width: 420
                text: about.backend ? about.backend.settingsPath : ""
                color: theme.textSecondary
                font.family: "monospace"
                font.pixelSize: theme.fontMeta
                elide: Text.ElideMiddle
                horizontalAlignment: Text.AlignRight
                ToolTip.visible: pathHover.hovered
                ToolTip.text: text

                HoverHandler {
                    id: pathHover
                }
            }
        }
    }
}
