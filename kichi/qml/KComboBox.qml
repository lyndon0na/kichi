import QtQuick
import QtQuick.Controls

// 统一的下拉框外观：圆角和输入框一致，只覆盖 background。
//
// contentItem / indicator / delegate 继续交给当前风格——它们负责弹层里的
// 高亮、键盘导航和箭头绘制，自己重写反而容易和系统主题脱节。
ComboBox {
    id: control

    KTheme { id: theme }

    implicitHeight: 36
    leftPadding: 12
    rightPadding: 12
    hoverEnabled: true

    // 自己画文字和箭头：Breeze 风格的 contentItem 是一个禁用的 TextField，
    // 颜色取自 Kirigami 主题（不是 QQC2 palette），在本应用里会变成
    // 「浅底上的浅字」，整只下拉框看起来是空的（2026-10-29 实测）。
    // 我们只用非可编辑下拉框，所以直接给一个普通 Label，颜色由 KTheme 控制。
    contentItem: Label {
        leftPadding: 12
        rightPadding: control.indicator.width + 8
        text: control.displayText
        color: control.enabled ? theme.textPrimary : theme.textMuted
        font.pixelSize: theme.fontBody
        verticalAlignment: Text.AlignVCenter
        elide: Text.ElideRight
    }

    indicator: NavIcon {
        x: control.mirrored ? control.leftPadding : control.width - width - control.rightPadding
        y: control.topPadding + (control.availableHeight - height) / 2
        implicitWidth: 14
        implicitHeight: 14
        name: "chevron-down"
        color: theme.textMuted
    }

    background: Rectangle {
        radius: theme.radiusControl
        color: control.hovered ? theme.surfaceHover : theme.surfaceRaised
        border.width: control.activeFocus ? 2 : 1
        border.color: control.activeFocus ? theme.accent
                    : control.hovered ? Qt.rgba(palette.text.r, palette.text.g,
                                                palette.text.b, 0.22)
                    : theme.hairline
    }
}
