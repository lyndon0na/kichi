import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 设置页 / 分组区块的小标题：左侧一根 3px 强调色竖条 + 14px 半粗文字。
//
// 以前各段只有一句 font.bold 的默认字号，标题和正文分不开；竖条让分组
// 在长页面里扫起来有节奏，也不必再加一条横贯整页的分割线。
RowLayout {
    id: title

    property string text: ""

    KTheme { id: theme }

    spacing: theme.gapSm

    Rectangle {
        Layout.alignment: Qt.AlignVCenter
        implicitWidth: 3
        implicitHeight: theme.fontSection
        radius: 1.5
        color: theme.accent
    }

    Label {
        text: title.text
        font.pixelSize: theme.fontSection
        font.weight: Font.DemiBold
        color: theme.textPrimary
    }

    Item {
        Layout.fillWidth: true
    }
}
