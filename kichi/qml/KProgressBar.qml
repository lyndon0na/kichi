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
// 系统文件改不了（升级会覆盖，也不是我们的包），所以这里自绘轨道和填充。
// 不能只覆盖 contentItem：org.kde.desktop 的 background 本身就会把轨道和进度
// 一起画出来，再叠一层 contentItem 会变成「粗胶囊压在细轨道上」。
//
// 用法与 ProgressBar 完全一致：from / to / value / indeterminate。
ProgressBar {
    id: bar

    KTheme { id: theme }

    // 虚线段长（与 breeze 的 14px 一致）
    readonly property int segment: 14
    /// 轨道和填充的固定高度：不跟随风格 background 的内部实现
    readonly property int grooveHeight: 8

    // 填充色：系统强调色 —— 圆角实条，不再描一圈比槽更深的边
    readonly property color fillColor: Qt.rgba(theme.accent.r, theme.accent.g,
                                               theme.accent.b, 0.85)

    implicitWidth: 200
    implicitHeight: grooveHeight
    leftInset: 0
    rightInset: 0
    topInset: 0
    bottomInset: 0
    leftPadding: 0
    rightPadding: 0
    topPadding: 0
    bottomPadding: 0

    background: Item {
        implicitWidth: 200
        implicitHeight: bar.grooveHeight

        Rectangle {
            width: parent.width
            height: bar.grooveHeight
            anchors.verticalCenter: parent.verticalCenter
            radius: height / 2
            color: theme.surfacePressed
        }
    }

    contentItem: Item {
        implicitWidth: 200
        implicitHeight: bar.grooveHeight
        clip: true

        // —— 确定进度：一条填充条 ——
        Rectangle {
            visible: !bar.indeterminate
            x: 0
            y: (parent.height - height) / 2
            height: bar.grooveHeight
            width: Math.max(0, bar.position) * parent.width
            radius: Math.min(width, height) / 2
            color: bar.fillColor
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
            y: (parent.height - height) / 2
            width: parent.width + 2 * bar.segment
            height: bar.grooveHeight
            clip: true

            Row {
                anchors.fill: parent
                spacing: bar.segment

                Repeater {
                    model: Math.max(1, Math.round(parent.width / (2 * bar.segment)))
                    delegate: Rectangle {
                        width: bar.segment
                        height: marquee.height
                        radius: Math.min(width, height) / 2
                        color: bar.fillColor
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
