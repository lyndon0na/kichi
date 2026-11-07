import QtQuick
import QtQuick.Controls

// 截断文字的统一提示：只有文字实际被省略且指针停在文字上时才显示。
//
// 文件行等位置会用整行 MouseArea 接管鼠标，子项的 HoverHandler 收不到悬停；
// 这些调用点通过 externallyHovered 传入按文字区域算出的悬停状态。
Label {
    id: control

    property string toolTipText: control.text
    property bool toolTipEnabled: true
    // 少数历史提示需要始终显示补充信息，不因文字未截断而消失。
    property bool showToolTipWhenUntruncated: false
    property bool externallyHovered: false
    property int toolTipDelay: 500

    // 判断用实际要显示的 ToolTip.text：调用点可以把它改写成别的说明（如所在文件夹），
    // 改写结果可能为空；只看 toolTipText 会在那时弹出一个没有字的空框。
    readonly property bool toolTipRequested: ToolTip.text.length > 0
        && (control.truncated || showToolTipWhenUntruncated)
        && (localHover.hovered || externallyHovered)

    ToolTip.visible: toolTipEnabled && toolTipRequested
    ToolTip.delay: toolTipDelay
    ToolTip.text: toolTipText

    HoverHandler {
        id: localHover
    }
}
