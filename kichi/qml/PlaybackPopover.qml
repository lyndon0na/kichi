import QtQuick
import QtQuick.Controls

// 播放窗口专用浮层：深色半透明表面 + 细白描边 + 10px 圆角。
//
// 不用系统 Menu：KDE 风格会把菜单画成浅色、带系统复选框和系统滚动条，
// 在黑色视频画面上很突兀。这里用一个自绘 Popup，行样式由调用方提供。
// 内容超过窗口高度时用应用统一的 KScrollBar（深色 palette 下是白色细条）。
Popup {
    id: popover

    /// 由子类在 openAbove() 前算好，包含内边距之外的内容高度
    property int bodyHeight: 0
    property int popupWidth: 260
    property int maxPopupHeight: 420

    /// 可滚动的中间内容
    default property alias items: column.data
    /// 固定头部 / 底部：不参与滚动，轨道再多也始终可见
    property alias headerItems: headerColumn.data
    property alias footerItems: footerColumn.data

    width: popupWidth
    height: Math.min(bodyHeight + topPadding + bottomPadding, maxPopupHeight)
    padding: 6
    topPadding: 6
    bottomPadding: 6
    modal: false
    focus: true
    closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside

    // 弹出层是独立窗口：显式给深色 palette，KScrollBar / 原生控件都跟着变深。
    palette.window: "#101010"
    palette.windowText: "#F2FFFFFF"
    palette.base: "#101010"
    palette.text: "#F2FFFFFF"
    palette.placeholderText: "#99FFFFFF"

    background: Rectangle {
        radius: 10
        color: "#EE101010"
        border.width: 1
        border.color: "#26FFFFFF"
    }

    contentItem: Item {
        id: content

        Column {
            id: headerColumn

            // 和中间轨道列同宽：右侧 10px 留给滚动条，勾选图标才能对齐。
            width: parent.width - 10
            anchors.top: parent.top
        }

        Flickable {
            id: flick

            anchors.top: headerColumn.bottom
            anchors.bottom: footerColumn.top
            width: parent.width
            contentHeight: column.implicitHeight
            boundsBehavior: Flickable.StopAtBounds
            clip: true
            interactive: contentHeight > height

            ScrollBar.vertical: KScrollBar {}

            Column {
                id: column

                // 右侧留 10px 给悬浮滚动条；不压住文字。
                width: flick.width - 10
                spacing: 0
            }
        }

        Column {
            id: footerColumn

            width: parent.width - 10
            anchors.bottom: parent.bottom
        }
    }

    /// 右对齐贴在锚点按钮上方 8px 处。
    /// Popup 只有 open()，没有 Menu 那种 popup(parent, x, y)，坐标自己算。
    function openAbove(anchor) {
        var origin = anchor.mapToItem(null, 0, 0)
        x = Math.round(origin.x + anchor.width - width)
        y = Math.round(origin.y - height - 8)
        open()
    }

    /// 把第 index 行滚进可视区（打开时定位当前轨道）。
    function ensureRowVisible(index, rowHeight) {
        if (!opened)
            return
        // 刚 open() 时内容列还没完成布局，此时写 contentY 会被夹回 0。
        if (flick.height <= 0 || flick.contentHeight <= 0) {
            Qt.callLater(function () { popover.ensureRowVisible(index, rowHeight) })
            return
        }
        var top = index * rowHeight
        var bottom = top + rowHeight
        var maxY = Math.max(0, flick.contentHeight - flick.height)
        if (top < flick.contentY)
            flick.contentY = Math.min(top, maxY)
        else if (bottom > flick.contentY + flick.height)
            flick.contentY = Math.min(bottom - flick.height, maxY)
    }
}
