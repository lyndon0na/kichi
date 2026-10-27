pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls

// 全应用唯一的进度条样式（见 docs/pitfalls.md 第 15 条）。
//
// 为什么不用裸 ProgressBar：KDE 的 qqc2-breeze-style 里，ProgressBar 的
// contentItem 用 Repeater 按「当前宽度」铺虚线，delegate 用 anchors.top /
// anchors.bottom 锚到 parent。进度条宽度一变（同一行右侧的百分比 / 速度文案
// 从 "9%" 变成 "10%"，整行重新布局），段数随之变化、delegate 重建，
// 重建中的 delegate 读到 parent 为 null，于是逐帧刷：
//   qrc:/qt/qml/org/kde/breeze/ProgressBar.qml:71: TypeError: Cannot read property 'top' of null
//   qrc:/qt/qml/org/kde/breeze/ProgressBar.qml:72: TypeError: Cannot read property 'bottom' of null
// 注意：即使 indeterminate 为 false 也照刷——那个虚线容器 visible: false
// 时宽度仍跟着走，Repeater 仍在维护。
//
// 系统文件改不了（升级会覆盖，也不是我们的包），所以这里只覆盖 contentItem；
// background（槽、圆角、高度、配色）仍旧交给当前风格，换主题不会脱节。
//
// 用法与 ProgressBar 完全一致：from / to / value / indeterminate。
ProgressBar {
    id: bar

    // 虚线段长（与 breeze 的 14px 一致）
    readonly property int segment: 14

    // 填充色：高亮色 55%（breeze 的观感），描边用高亮色本身
    readonly property color fillColor: Qt.rgba(bar.palette.highlight.r, bar.palette.highlight.g,
                                               bar.palette.highlight.b, 0.55)

    contentItem: Item {
        implicitWidth: 200
        // 槽高跟随当前风格的 background，换风格不高低不一（取不到时退回 6px）
        implicitHeight: bar.implicitBackgroundHeight > 0 ? bar.implicitBackgroundHeight : 6
        clip: true

        // —— 确定进度：一条填充条 ——
        Rectangle {
            visible: !bar.indeterminate
            x: 0
            y: 0
            height: parent.height
            width: Math.max(0, bar.position) * parent.width
            radius: height / 2
            color: bar.fillColor
            border.width: 1
            border.color: bar.palette.highlight
        }

        // —— 不确定进度：一条虚线向左滚动 ——
        //
        // 段数仍按宽度算（变宽变窄都不留空），但 delegate 只用显式尺寸和
        // 稳定 id（bar / marquee），不再 anchors 到可能为空的 parent ——
        // 这正是上面那两条告警的根因。
        Item {
            id: marquee
            visible: bar.indeterminate
            x: -2 * bar.segment
            y: 0
            width: parent.width + 2 * bar.segment
            height: parent.height
            clip: true

            Row {
                anchors.fill: parent
                spacing: bar.segment

                Repeater {
                    model: Math.max(1, Math.round(parent.width / (2 * bar.segment)))
                    delegate: Rectangle {
                        width: bar.segment
                        height: marquee.height
                        radius: height / 2
                        color: bar.fillColor
                        border.width: 1
                        border.color: bar.palette.highlight
                    }
                }
            }

            NumberAnimation on x {
                from: -2 * bar.segment
                to: 0
                duration: 3600
                loops: Animation.Infinite
                running: bar.indeterminate
            }
        }
    }
}
