import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "../components"
import "../core"

// 设置页「关于」段：只读的本机存储位置。
ColumnLayout {
    id: about

    property var backend: null

    readonly property KTheme theme: KTheme { parent: about }

    spacing: theme.gapXl

    KSettingsGroup {
        title: qsTr("存储位置")
        Layout.fillWidth: true

        KSettingsRow {
            title: qsTr("设置文件")

            ElidedLabel {
                id: settingsPath
                objectName: "settingsPath"
                width: 320
                text: about.backend ? about.backend.settingsPath : ""
                color: theme.textSecondary
                font.family: "monospace"
                font.pixelSize: theme.fontMeta
                elide: Text.ElideMiddle
                horizontalAlignment: Text.AlignRight
            }
        }
    }
}
