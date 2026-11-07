import QtQuick
import QtQuick.Controls
import "../core"

// 统一的输入框外观：圆角 8、极浅描边、聚焦时换成强调色描边。
//
// 行为与 TextField 完全一致（TextInput / 密码模式 / accepted 信号 / Keys
// 附加对象都照常可用），只覆盖 background 和默认内边距。风格自带的圆角
// 在 Breeze / Basic 下不一样，搜索框和设置页并排时看得出来。
TextField {
    id: control

    KTheme { id: theme }

    implicitHeight: 36
    leftPadding: 12
    rightPadding: 12
    topPadding: 8
    bottomPadding: 8
    font.pixelSize: theme.fontBody
    color: theme.textPrimary
    placeholderTextColor: theme.textMuted
    hoverEnabled: true

    background: Rectangle {
        radius: theme.radiusControl
        color: theme.surfaceRaised
        border.width: control.activeFocus ? 2 : 1
        border.color: control.activeFocus ? theme.accent
                    : control.hovered ? Qt.rgba(palette.text.r, palette.text.g,
                                                palette.text.b, 0.22)
                    : theme.hairline
    }
}
