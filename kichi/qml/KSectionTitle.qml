import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 设置页分组标题：小号灰字 + 一条拉满剩余宽度的极细分隔线。
//
// 卡片式设置里分组不再靠底色分区，这一行就是唯一的结构提示；
// 字号比正文小一档，免得和行标题抢注意力（2026-11-06 用户指定卡片式样式）。
RowLayout {
    id: title

    property string text: ""

    KTheme { id: theme }

    spacing: theme.gapMd

    Label {
        text: title.text
        font.pixelSize: theme.fontCaption
        font.weight: Font.Medium
        font.letterSpacing: 0.6
        color: theme.textMuted
    }

    Rectangle {
        Layout.fillWidth: true
        Layout.alignment: Qt.AlignVCenter
        implicitHeight: 1
        color: theme.divider
    }
}
