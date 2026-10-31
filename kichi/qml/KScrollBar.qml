import QtQuick
import QtQuick.Controls

// 全应用唯一的滚动条样式（见 docs/pitfalls.md 第 13 条）。
//
// 用法只有一种：`ScrollBar.vertical: KScrollBar {}`，页面里不再传任何参数。
// 宽度 10px = 内容要让出的车道；滑块 6px 居中在车道里。
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
