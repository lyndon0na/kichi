import QtQuick
import QtQuick.Shapes

// 单色线稿图标：侧栏导航 + 文件类型（FileIcon.qml 复用本组件）。
//
// 为什么不用 IconImage + 系统图标主题：主题里缺一个名字就整块空白，
// 而且彩色图标在侧栏缩到 18px 会糊。这里用 24×24 单位画布描边绘制，
// 颜色跟随调用方（选中/悬停都能变），不依赖任何图标主题。
//
// 路径来自 Tabler Icons（MIT）：https://github.com/tabler/tabler-icons
Item {
    id: icon

    /// folder / clock / history / star / share / transfer / upload / download / trash / settings / logout
    /// 以及文件类型：file / video / music / photo / pdf / document /
    /// spreadsheet / presentation / archive / code / subtitle / application / torrent
    property string name: ""
    property color color: "transparent"
    /// 星标等实心状态用；其它图标保持线稿。
    property bool filled: false

    implicitWidth: 18
    implicitHeight: 18

    readonly property var pathData: ({
        folder: "M5 4h4l3 3h7a2 2 0 0 1 2 2v8a2 2 0 0 1 -2 2h-14a2 2 0 0 1 -2 -2v-11a2 2 0 0 1 2 -2",
        clock: "M12 12m-9 0a9 9 0 1 0 18 0a9 9 0 1 0 -18 0 M12 7v5l3 3",
        history: "M12 8l0 4l2 2 M3.05 11a9 9 0 1 1 .5 4m-.5 5v-5h5",
        star: "M12 17.75l-6.172 3.245l1.179 -6.873l-5 -4.867l6.9 -1l3.086 -6.253l3.086 6.253l6.9 1l-5 4.867l1.179 6.873z",
        download: "M19 18a3.5 3.5 0 0 0 0 -7h-1a5 5 0 0 0 -9.3 -2a4.6 4.6 0 0 0 -2.7 8.4"
                  + " M12 13v8 M9 18l3 3l3 -3",
        upload: "M19 18a3.5 3.5 0 0 0 0 -7h-1a5 5 0 0 0 -9.3 -2a4.6 4.6 0 0 0 -2.7 8.4"
                + " M12 21v-8 M9 16l3 -3l3 3",
        // Tabler「arrows-exchange」：左右双箭头，和云朵下载区分开
        transfer: "M7 10h14l-4 -4 M17 14h-14l4 4",
        share: "M6 12m-3 0a3 3 0 1 0 6 0a3 3 0 1 0 -6 0"
               + " M18 6m-3 0a3 3 0 1 0 6 0a3 3 0 1 0 -6 0"
               + " M18 18m-3 0a3 3 0 1 0 6 0a3 3 0 1 0 -6 0"
               + " M8.7 10.7l6.6 -3.4 M8.7 13.3l6.6 3.4",
        settings: "M10.325 4.317c.426 -1.756 2.924 -1.756 3.35 0a1.724 1.724 0 0 0 2.573 1.066"
                  + "c1.543 -.94 3.31 .826 2.37 2.37a1.724 1.724 0 0 0 1.065 2.572c1.756 .426 1.756 2.924 0 3.35"
                  + "a1.724 1.724 0 0 0 -1.066 2.573c.94 1.543 -.826 3.31 -2.37 2.37"
                  + "a1.724 1.724 0 0 0 -2.572 1.065c-.426 1.756 -2.924 1.756 -3.35 0"
                  + "a1.724 1.724 0 0 0 -2.573 -1.066c-1.543 .94 -3.31 -.826 -2.37 -2.37"
                  + "a1.724 1.724 0 0 0 -1.065 -2.572c-1.756 -.426 -1.756 -2.924 0 -3.35"
                  + "a1.724 1.724 0 0 0 1.066 -2.573c-.94 -1.543 .826 -3.31 2.37 -2.37"
                  + "c1 .608 2.296 .07 2.572 -1.065z"
                  + " M9 12a3 3 0 1 0 6 0a3 3 0 1 0 -6 0",
        trash: "M4 7l16 0 M10 11l0 6 M14 11l0 6 M5 7l1 12a2 2 0 0 0 2 2h8a2 2 0 0 0 2 -2l1 -12"
               + " M9 7v-3a1 1 0 0 1 1 -1h4a1 1 0 0 1 1 1v3",
        restore: "M9 14l-4 -4l4 -4 M5 10h11a4 4 0 1 1 0 8h-1",
        logout: "M14 8v-2a2 2 0 0 0 -2 -2h-7a2 2 0 0 0 -2 2v12a2 2 0 0 0 2 2h7a2 2 0 0 0 2 -2v-2"
                + " M9 12h12 M18 15l3 -3l-3 -3",
        // 普通文件：右上角折角
        file: "M14 3v4a1 1 0 0 0 1 1h4"
              + " M17 21h-10a2 2 0 0 1 -2 -2v-14a2 2 0 0 1 2 -2h7l5 5v11a2 2 0 0 1 -2 2z",
        // PDF：纸张 + 清楚的 P/D/F 字形
        pdf: "M14 3v4a1 1 0 0 0 1 1h4"
             + " M5 12v-7a2 2 0 0 1 2 -2h7l5 5v4"
             + " M5 18h1.5a1.5 1.5 0 0 0 0 -3h-1.5v6"
             + " M17 18h2 M20 15h-3v6"
             + " M11 15v6h1a2 2 0 0 0 2 -2v-2a2 2 0 0 0 -2 -2h-1",
        // 文档 / 纯文本：纸张 + 三行文字
        document: "M14 3v4a1 1 0 0 0 1 1h4"
                  + " M17 21h-10a2 2 0 0 1 -2 -2v-14a2 2 0 0 1 2 -2h7l5 5v11a2 2 0 0 1 -2 2"
                  + " M9 9h1 M9 13h6 M9 17h6",
        // 表格：纸张 + 网格
        spreadsheet: "M14 3v4a1 1 0 0 0 1 1h4"
                     + " M17 21h-10a2 2 0 0 1 -2 -2v-14a2 2 0 0 1 2 -2h7l5 5v11a2 2 0 0 1 -2 2"
                     + " M8 11h8v7h-8z M8 15h8 M11 11v7",
        // 演示文稿：投影屏 + 折线图
        presentation: "M3 4h18 M4 4v10a2 2 0 0 0 2 2h12a2 2 0 0 0 2 -2v-10"
                      + " M12 16v4 M9 20h6 M8 12l3 -3l2 2l3 -3",
        // 压缩包：纸张 + 拉链
        archive: "M6 20.735a2 2 0 0 1 -1 -1.735v-14a2 2 0 0 1 2 -2h7l5 5v11a2 2 0 0 1 -2 2h-1"
                 + " M11 17a2 2 0 0 1 2 2v2a1 1 0 0 1 -1 1h-2a1 1 0 0 1 -1 -1v-2a2 2 0 0 1 2 -2"
                 + " M11 5h-1 M13 7h-1 M11 9h-1 M13 11h-1 M11 13h-1 M13 15h-1",
        // 视频：摄像机，缩到 18px 也比「胶片格」清楚
        video: "M15 10l4.553 -2.276a1 1 0 0 1 1.447 .894v6.764a1 1 0 0 1 -1.447 .894l-4.553 -2.276v-4z"
               + " M3 6m0 2a2 2 0 0 1 2 -2h8a2 2 0 0 1 2 2v8a2 2 0 0 1 -2 2h-8a2 2 0 0 1 -2 -2z",
        music: "M8 17a3 3 0 1 0 6 0a3 3 0 1 0 -6 0 M14 17v-13h4",
        photo: "M15 8h.01"
               + " M3 6a3 3 0 0 1 3 -3h12a3 3 0 0 1 3 3v12a3 3 0 0 1 -3 3h-12a3 3 0 0 1 -3 -3v-12z"
               + " M3 16l5 -5c.928 -.893 2.072 -.893 3 0l5 5"
               + " M14 14l1 -1c.928 -.893 2.072 -.893 3 0l3 3",
        // 代码：左右尖括号 + 斜杠
        code: "M7 8l-4 4l4 4 M17 8l4 4l-4 4 M14 4l-4 16",
        // 字幕：带两行文字的字幕框
        subtitle: "M18 5a3 3 0 0 1 3 3v8a3 3 0 0 1 -3 3h-12a3 3 0 0 1 -3 -3v-8a3 3 0 0 1 3 -3h12"
                  + " M7 15h5 M15 15h2 M17 12h-3 M11 12h-1",
        // 安装包 / 应用：包裹盒
        application: "M12 3l8 4.5v9l-8 4.5l-8 -4.5v-9z"
                     + " M12 12l8 -4.5 M12 12v9 M12 12l-8 -4.5 M16 5.25l-8 4.5",
        // 种子：磁铁
        torrent: "M4 13v-8a2 2 0 0 1 2 -2h1a2 2 0 0 1 2 2v8a2 2 0 0 0 6 0v-8a2 2 0 0 1 2 -2h1a2 2 0 0 1 2 2v8a8 8 0 0 1 -16 0"
                 + " M4 8h5 M15 8h4",
        // 工具条用：刷新 / 新建 / 上一级 / 搜索 / 关闭
        // 刷新用「一段弧 + 一个箭头」（Tabler reload）：双箭头的 refresh 在 14px 下
        // 两个箭头会挤成一团墨点，看不出是刷新
        refresh: "M19.933 13.041a8 8 0 1 1 -9.925 -8.788c3.899 -1 7.935 1.007 9.425 4.747"
                 + " M20 4v5h-5",
        plus: "M12 5l0 14 M5 12l14 0",
        "arrow-up": "M12 5l0 14 M18 11l-6 -6l-6 6",
        search: "M10 10m-7 0a7 7 0 1 0 14 0a7 7 0 1 0 -14 0 M21 21l-6 -6",
        x: "M18 6l-12 12 M6 6l12 12",
        check: "M5 12l5 5l9 -9",
        // 文件页视图切换
        list: "M9 6l11 0 M9 12l11 0 M9 18l11 0 M5 6l0 .01 M5 12l0 .01 M5 18l0 .01",
        "layout-grid": "M4 4h6v6h-6z M14 4h6v6h-6z M4 14h6v6h-6z M14 14h6v6h-6z",
        // 下拉框箭头
        "chevron-down": "M6 9l6 6l6 -6",
        "chevron-right": "M9 6l6 6l-6 6"
    })

    Shape {
        anchors.fill: parent

        // Shape 默认不抗锯齿：1.6px 的细描边在非整数像素位置会出毛刺。
        // 线稿图标全靠描边，必须开。
        antialiasing: true

        // 路径按 24 单位画布书写，这里缩放到实际边长
        transform: Scale {
            xScale: icon.width / 24
            yScale: icon.height / 24
        }

        ShapePath {
            fillColor: icon.filled ? icon.color : "transparent"
            strokeColor: icon.color
            // 描边宽度按图标尺寸成比例（视觉上 = 边长的 9%：18px 约 1.6px、24px 约 2.2px）。
            // 写死像素会让 14px 的小图标描边占 11%，又粗又毛。
            strokeWidth: 2.16
            capStyle: ShapePath.RoundCap
            joinStyle: ShapePath.RoundJoin

            PathSvg {
                path: icon.pathData[icon.name] || ""
            }
        }
    }
}
