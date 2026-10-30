import QtQuick

// 媒体遮罩上的高对比加载环。KDE 的 BusyIndicator 使用系统 symbolic
// 图标，图标颜色会被浅色主题画成深色；这里固定用传入前景色绘制圆点环。
Item {
    id: root

    property color color: "#ffffff"
    property bool running: true

    implicitWidth: 16
    implicitHeight: 16

    readonly property real _radius: Math.max(0, Math.min(width, height) / 2 - 2)
    readonly property real _dotSize: Math.max(2, Math.min(width, height) / 6)

    Repeater {
        model: 8

        delegate: Rectangle {
            required property int index

            width: root._dotSize
            height: width
            radius: width / 2
            color: root.color
            opacity: 0.18 + 0.82 * (index + 1) / 8
            x: root.width / 2
               + root._radius * Math.cos(index * Math.PI / 4)
               - width / 2
            y: root.height / 2
               + root._radius * Math.sin(index * Math.PI / 4)
               - height / 2
        }
    }

    transformOrigin: Item.Center

    RotationAnimator on rotation {
        from: 0
        to: 360
        duration: 900
        loops: Animation.Infinite
        running: root.running && root.visible
    }
}
