import QtQuick
import QtQuick.Controls

// 页面级尺寸常量。
//
// 可滚视图铺到窗口右缘，滚动条悬浮在页面右侧留白里，不占布局：
// 条宽不写死在这里，直接量 KScrollBar 的宽度，改样式只改那一个文件。
QtObject {
    id: metrics

    // 只用来读宽度，不显示、不占位
    readonly property KScrollBar probe: KScrollBar {}

    // 滚动条悬浮宽度：点击层避让和网格列数估算用，不是内容要让出的宽度
    readonly property int scrollBarSlot: metrics.probe.implicitWidth

    // 传输 / 离线 / 设置页的内容边距（右侧为 0，留给滚动条贴窗口边）
    readonly property int pageMargin: 12
}
