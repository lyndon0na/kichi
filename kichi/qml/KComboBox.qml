import QtQuick
import QtQuick.Controls

// 统一的下拉框外观：圆角和输入框一致，只覆盖视觉部件。
//
// delegate / popup 继续交给当前风格，保留弹层高亮和键盘导航。
ComboBox {
    id: control

    KTheme { id: theme }

    implicitHeight: 36
    leftPadding: 12
    rightPadding: 12
    hoverEnabled: true

    // 自己画文字和箭头：Breeze 风格的 contentItem 颜色取自 Kirigami 主题
    // （不是 QQC2 palette），在本应用里会变成「浅底上的浅字」。
    //
    // 这里必须是 TextInput，不能换成 Label：org.kde.desktop 的 ComboBox 会无条件
    // 把 contentItem 当成 TextInput/TextEdit，给 MobileCursor 调
    // positionToRectangle() 并读取 selectionStart；Label 会直接报 TypeError。
    contentItem: TextInput {
        leftPadding: 12
        rightPadding: control.indicator.width + 8
        text: control.displayText
        readOnly: true
        selectByMouse: false
        activeFocusOnTab: false
        color: control.enabled ? theme.textPrimary : theme.textMuted
        font.pixelSize: theme.fontBody
        verticalAlignment: Text.AlignVCenter
        clip: true
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
