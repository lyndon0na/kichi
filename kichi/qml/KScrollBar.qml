import QtQuick
import QtQuick.Controls

// 全应用唯一的滚动条样式（见 docs/pitfalls.md 第 13 条）。
//
// 普通页面只写 `ScrollBar.vertical: KScrollBar {}`，不传参数。
// ScrollView 内替换任一方向的默认滚动条时，必须像 NewTaskDialog 一样显式挂 parent
// 和 anchors，并把车道补进 viewport；否则桌面风格的原定位绑定会一起被替换掉。
// 竖向车道 10px = 滑块 6px + 两侧各 2px。
// 离线任务输入框的横向条不要整条轨道线，只自绘 6px 椭圆滑块。
// 列表 / 表格始终保留车道；网格只在确实需要滚动时保留。
// 这里只管画：内容装得下时 policy 自动隐藏，装不下就一直淡淡在位。
ScrollBar {
    id: bar

    // 车道宽度。UiMetrics.scrollBarSlot 直接量这个值，改宽度只改这里
    implicitWidth: 10
    // 装得下就不画；不搞「滚动后淡出」，静止时四页观感一致
    policy: ScrollBar.AsNeeded
    // 上万条时滑块仍抓得住
    minimumSize: 0.08
    // 车道 10 = 滑块 6 + 左右各 2
    padding: 2

    contentItem: Rectangle {
        implicitWidth: 6
        implicitHeight: 6
        radius: 3
        // 跟主题走：浅色 / 深色下都自动换成 windowText，不写死色值
        color: Qt.rgba(palette.windowText.r, palette.windowText.g, palette.windowText.b,
                       bar.pressed ? 0.55 : bar.hovered ? 0.40 : 0.26)
    }
}
