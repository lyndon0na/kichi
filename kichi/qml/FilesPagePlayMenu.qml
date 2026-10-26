import QtQuick
import QtQuick.Controls

// 清晰度菜单：固定四项（原画 / 1080P / 720P / 480P），点播放立刻弹出，不等网络。
//
// 只有用户点了某一项之后，后端才去请求档位与直链；请求回来发现没有那一档会回退，
// 并在状态栏说明实际播的是哪一档。所以这里的行**永远是可点的**，
// 不用（也无法）预先知道文件有没有这一档。
//
// 从 FilesPage.qml 拆出来：那是页面结构，这里是播放相关的弹层。
// 页面只需要在拿到点击位置时调 openAt()。
//
// 定位方式：Popup 的 x/y 和 popup(pos) 都是**相对父项**的。菜单默认挂在
// 所在页面下，页面在窗口里偏了多少，菜单就整体偏多少（实测双击后低好几行）。
// 所以这里把父项固定成窗口的 overlay：它铺满窗口、原点就在窗口左上角，
// 于是「窗口坐标」和「相对父项坐标」变成同一个东西，锚点怎么算都不会偏。
Menu {
    id: menu

    parent: Overlay.overlay

    property var backend: null
    // 锚点（**窗口**坐标系，通常是目标行的行底中点）
    property point anchorPoint: Qt.point(0, 0)

    // 宽度只在这里定，子项跟着撑满（与右键菜单同一条纪律，见 pitfalls 第 9 条）。
    // 不给子项写 width：逐项写死 width 会让高亮条溢出容器的圆角。
    // 标题行是固定文案「选择清晰度」，宽度只需容纳档位文字（含「（需要额外播放配额）」）。
    width: 200

    // 行高显式写死。Menu 在 Windows / Fusion 这类带菜单主题的风格下，每个
    // MenuItem 自带上下 padding，按默认值算出来每行 50 多像素，整张菜单比列表行还高。
    // 这里按「字号 + 上下 5」收紧（约 25px），保留点按面积。
    // 注意 padding 会叠加：Menu 自身的上下 padding 也要一起收，否则菜单上下各留一大块空白。
    // 字号取 FontMetrics：跟应用默认字体走，不写死像素（Label 的 font 要等窗格建好才有值）
    readonly property FontMetrics metrics: FontMetrics {
        font: menu.font
    }
    padding: 2

    // 在窗口坐标系里弹出：和右键菜单同一套位置语义——菜单左上角落在点击处，
    // 竖直略偏下 2px 免得压住光标。超出窗口时由 Popup 自己的 margins
    // （Menu 默认 0）夹回窗口内，不用手工算边界。
    function openAt(point) {
        anchorPoint = point
        popup(Qt.point(point.x, point.y + 2))
    }

    // 用户点菜单外面关掉时，把后端的档位一起清掉（避免残留的直链）
    onClosed: menu.backend ? menu.backend.cancelPlay() : undefined

    // 标题行：只说「这是干什么用的」，不显示文件名（文件名列表里本来就有，
    // 长文件名还会把菜单撑变形）。Menu 自身只设 contentItem，不用老 Qt 版本没有的属性
    MenuItem {
        height: Math.max(24, menu.metrics.height + 10)
        topPadding: 0
        bottomPadding: 0
        contentItem: Label {
            leftPadding: 12
            rightPadding: 12
            topPadding: 0
            bottomPadding: 0
            text: qsTr("选择清晰度")
            elide: Text.ElideRight
            color: menu.palette.placeholderText
            verticalAlignment: Text.AlignVCenter
        }
        enabled: false
    }

    MenuSeparator {
        // 分隔线也跟着收紧，默认带一大段上下留白
        topPadding: 0
        bottomPadding: 0
        contentItem: Rectangle {
            implicitHeight: 1
            color: menu.palette.mid
        }
    }

    Repeater {
        model: menu.backend ? menu.backend.playMenu : []

        // 行高在 delegate 里自己算：静态检查对这里的跨层派生属性会报 not found
        delegate: MenuItem {
            height: Math.max(24, metrics.height + 10)
            topPadding: 0
            bottomPadding: 0
            text: modelData.label
                  + (modelData.original ? qsTr(" · 原文件") : "")
            onClicked: menu.backend.playSelected(index)
        }
    }

    // playMenu 一有内容就弹（内容是固定的四项，随时可以弹）
    Connections {
        target: menu.backend

        function onChanged() {
            // 后端清空了菜单（选中开播 / 出错 / 登出），界面上的菜单也要收起
            if (!menu.backend || menu.backend.playMenu.length === 0) {
                if (menu.visible)
                    menu.close()
                return
            }
            if (!menu.visible)
                menu.openAt(menu.anchorPoint)
        }
    }
}
