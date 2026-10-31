import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 全应用统一的按钮外观。
//
// 不用风格自带的 Button 背景：Breeze / Basic 两套风格下圆角、悬停、按下
// 各不相同，设置页和应用其它地方的按钮看起来不是一家人。这里只覆盖
// background / contentItem，点击、enabled、焦点、键盘行为仍由 Button 负责。
//
// variant：
// - "secondary"（默认）：浅色表面 + 极浅描边，用于普通操作
// - "primary"：系统强调色实底，用于每个视图里唯一的主操作（登录 / 应用 / 提交）
// - "ghost"：无底，仅悬停时出现淡淡一层，用于「恢复默认」「删除」等次要操作
// - "danger"：危险操作
// - "dangerOutline"：低强调危险操作，保留危险语义但不抢页面视觉重心
Button {
    id: control

    property string variant: "secondary"
    /// 可选线稿图标（NavIcon 的名字），留空则只有文字
    property string iconName: ""
    property int iconSize: 16
    /// 忙碌时用同尺寸转圈替代图标，文字和按钮宽度保持不变
    property bool busy: false
    // 调用处已有说明性 ToolTip 时关掉文字自身的截断提示，避免同时弹两层。
    property bool showTextToolTip: true

    KTheme { id: theme }

    implicitHeight: 34
    leftPadding: 14
    rightPadding: 14
    topPadding: 6
    bottomPadding: 6
    font.pixelSize: theme.fontBody
    font.weight: Font.Medium
    // 禁用态整只按钮一起变淡（文字 / 图标 / 底都跟着走）
    opacity: enabled ? 1 : 0.45

    background: Rectangle {
        radius: theme.radiusControl
        color: {
            if (control.variant === "primary") {
                if (control.down)
                    return Qt.darker(theme.accent, 1.08)
                return control.hovered ? Qt.lighter(theme.accent, 1.10) : theme.accent
            }
            if (control.variant === "danger") {
                if (control.down)
                    return Qt.darker(theme.danger, 1.08)
                return control.hovered ? Qt.lighter(theme.danger, 1.08) : theme.danger
            }
            if (control.variant === "dangerOutline") {
                if (control.down)
                    return theme.softTint(theme.danger, 0.14)
                return control.hovered ? theme.softTint(theme.danger, 0.08)
                                       : theme.surfaceClear
            }
            if (control.variant === "ghost") {
                if (control.down)
                    return theme.surfacePressed
                return control.hovered ? theme.surfaceHover : theme.surfaceClear
            }
            if (control.down)
                return theme.surfacePressed
            return control.hovered ? theme.surfaceHover : theme.surfaceRaised
        }
        border.width: control.variant === "secondary"
                      || control.variant === "dangerOutline" ? 1 : 0
        border.color: control.visualFocus ? theme.accent
                    : control.variant === "dangerOutline"
                      ? theme.softTint(theme.danger, 0.34)
                    : theme.hairline
    }

    contentItem: RowLayout {
        spacing: 8

        BusyIndicator {
            objectName: "buttonBusyIndicator"
            visible: control.busy
            running: control.busy
            padding: 0
            implicitWidth: control.iconSize
            implicitHeight: control.iconSize
            Layout.preferredWidth: control.iconSize
            Layout.preferredHeight: control.iconSize
            Layout.alignment: Qt.AlignVCenter
        }

        NavIcon {
            visible: !control.busy && control.iconName.length > 0
            implicitWidth: control.iconSize
            implicitHeight: control.iconSize
            Layout.alignment: Qt.AlignVCenter
            name: control.iconName
            color: {
                if (control.variant === "primary")
                    return theme.onAccent
                if (control.variant === "danger")
                    return theme.onDanger
                if (control.variant === "dangerOutline")
                    return theme.danger
                return control.variant === "ghost" ? theme.textSecondary : theme.textPrimary
            }
            opacity: control.variant === "ghost" ? 0.8 : 1
        }

        ElidedLabel {
            objectName: "buttonText"
            Layout.fillWidth: true
            text: control.text
            toolTipEnabled: control.showTextToolTip
            font: control.font
            horizontalAlignment: Text.AlignHCenter
            verticalAlignment: Text.AlignVCenter
            elide: Text.ElideRight
            color: {
                if (control.variant === "primary")
                    return theme.onAccent
                if (control.variant === "danger")
                    return theme.onDanger
                if (control.variant === "dangerOutline")
                    return theme.danger
                return control.variant === "ghost" ? theme.textSecondary : theme.textPrimary
            }
        }
    }
}
