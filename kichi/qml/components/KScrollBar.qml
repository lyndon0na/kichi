import QtQuick
import QtQuick.Controls

// 全应用唯一的滚动条样式。
//
// 普通页面只写 `ScrollBar.vertical: KScrollBar {}`，不传参数。
// ScrollView 内替换任一方向的默认滚动条时，必须像 NewTaskDialog 一样显式挂 parent
// 和 anchors，并把车道补进 viewport；否则桌面风格的原定位绑定会一起被替换掉。
// 竖向条宽 10px = 滑块 6px + 两侧各 2px，悬浮在页面右侧 12px 留白里，不占布局。
// 继承来的 KDE 风格背景会画整条轨道线和外框，这里统一 background: null，只留滑块。
ScrollBar {
    id: bar

    // 悬浮条宽度。UiMetrics.scrollBarSlot 直接量这个值，改宽度只改这里
    implicitWidth: 10
    // 装得下就不画；装不下就一直淡淡在位，不做滚动后淡出
    policy: ScrollBar.AsNeeded
    // 上万条时滑块仍抓得住
    minimumSize: 0.08
    // KDE 风格把四边内边距绑到自己的样式槽位上，只写 `padding` 在真实界面里
    // 不生效（实测滑块永远是整条 10px）；必须直接写具体边距。
    // 竖向条左右各让 2，滑块本体成 6px；横向条不缩放滑块。
    leftPadding: horizontal ? 0 : 2
    rightPadding: horizontal ? 0 : 2
    topPadding: 0
    bottomPadding: 0
    // KDE 原生背景会画轨道线，必须清掉
    background: null

    contentItem: Rectangle {
        implicitWidth: 6
        implicitHeight: 6
        radius: 3
        // 跟主题走：浅色 / 深色下都自动换成 windowText，不写死色值
        color: Qt.rgba(palette.windowText.r, palette.windowText.g, palette.windowText.b,
                       bar.pressed ? 0.55 : bar.hovered ? 0.40 : 0.26)
    }
}
