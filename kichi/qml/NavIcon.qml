import QtQuick
import QtQuick.Shapes

// 侧栏导航用的单色线稿图标。
//
// 为什么不用 IconImage + 系统图标主题：主题里缺一个名字就整块空白，
// 而且彩色图标在侧栏缩到 18px 会糊。这里用 24×24 单位画布描边绘制，
// 颜色跟随调用方（选中/悬停都能变），不依赖任何图标主题。
//
// 路径来自 Tabler Icons（MIT）：https://github.com/tabler/tabler-icons
Item {
    id: icon

    /// folder / transfer / download / settings / logout
    property string name: ""
    property color color: "transparent"

    implicitWidth: 18
    implicitHeight: 18

    readonly property var pathData: ({
        folder: "M5 4h4l3 3h7a2 2 0 0 1 2 2v8a2 2 0 0 1 -2 2h-14a2 2 0 0 1 -2 -2v-11a2 2 0 0 1 2 -2",
        download: "M19 18a3.5 3.5 0 0 0 0 -7h-1a5 5 0 0 0 -9.3 -2a4.6 4.6 0 0 0 -2.7 8.4"
                  + " M12 13v8 M9 18l3 3l3 -3",
        // Tabler「arrows-exchange」：左右双箭头，和云朵下载区分开
        transfer: "M7 10h14l-4 -4 M17 14h-14l4 4",
        settings: "M10.325 4.317c.426 -1.756 2.924 -1.756 3.35 0a1.724 1.724 0 0 0 2.573 1.066"
                  + "c1.543 -.94 3.31 .826 2.37 2.37a1.724 1.724 0 0 0 1.065 2.572c1.756 .426 1.756 2.924 0 3.35"
                  + "a1.724 1.724 0 0 0 -1.066 2.573c.94 1.543 -.826 3.31 -2.37 2.37"
                  + "a1.724 1.724 0 0 0 -2.572 1.065c-.426 1.756 -2.924 1.756 -3.35 0"
                  + "a1.724 1.724 0 0 0 -2.573 -1.066c-1.543 .94 -3.31 -.826 -2.37 -2.37"
                  + "a1.724 1.724 0 0 0 -1.065 -2.572c-1.756 -.426 -1.756 -2.924 0 -3.35"
                  + "a1.724 1.724 0 0 0 1.066 -2.573c-.94 -1.543 .826 -3.31 2.37 -2.37"
                  + "c1 .608 2.296 .07 2.572 -1.065z"
                  + " M9 12a3 3 0 1 0 6 0a3 3 0 1 0 -6 0",
        logout: "M14 8v-2a2 2 0 0 0 -2 -2h-7a2 2 0 0 0 -2 2v12a2 2 0 0 0 2 2h7a2 2 0 0 0 2 -2v-2"
                + " M9 12h12 M18 15l3 -3l-3 -3"
    })

    Shape {
        anchors.fill: parent

        // 路径按 24 单位画布书写，这里缩放到实际边长
        transform: Scale {
            xScale: icon.width / 24
            yScale: icon.height / 24
        }

        ShapePath {
            fillColor: "transparent"
            strokeColor: icon.color
            // 描边宽度要先除以缩放比，视觉上才恒为 ~1.6px，不随尺寸变粗变细
            strokeWidth: 1.6 * 24 / icon.width
            capStyle: ShapePath.RoundCap
            joinStyle: ShapePath.RoundJoin

            PathSvg {
                path: icon.pathData[icon.name] || ""
            }
        }
    }
}
