import QtQuick
import QtQuick.Controls

// 统一的图标按钮：无边框，悬停时出现一层圆角淡底。
//
// 页头的刷新 / 新建、面包屑的上一级、搜索框里的清空与放大镜都用它，
// 目的是让所有工具按钮的悬停面积、圆角和图标尺寸一致（不再各写一套
// ToolButton + 自定义 background）。提示文字仍在调用处用 ToolTip 设置。
ToolButton {
    id: control

    property string iconName: ""
    /// 实心图标（播放 / 暂停这类媒体控制）
    property bool iconFilled: false
    property int iconSize: 14
    /// 图标透明度：工具条图标默认不抢正文，传 0.65
    property real iconOpacity: 0.65
    property color iconColor: palette.text

    KTheme { id: theme }

    implicitWidth: 30
    implicitHeight: 30
    padding: 0

    background: Rectangle {
        radius: theme.radiusSmall
        color: !control.enabled ? theme.surfaceClear
             : control.checked ? theme.accentGhost
             : control.down ? theme.surfacePressed
             : control.hovered ? theme.surfaceHover
             : theme.surfaceClear
    }

    contentItem: Item {
        implicitWidth: control.iconSize
        implicitHeight: control.iconSize

        NavIcon {
            anchors.centerIn: parent
            width: control.iconSize
            height: control.iconSize
            name: control.iconName
            filled: control.iconFilled
            color: control.checked ? theme.accent : control.iconColor
            opacity: control.enabled ? (control.checked ? 1 : control.iconOpacity)
                                     : control.iconOpacity * 0.4
        }
    }
}
