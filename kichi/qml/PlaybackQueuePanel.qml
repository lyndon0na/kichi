import QtQuick
import QtQuick.Controls

// 播放窗口的连播队列浮层：当前文件夹里的可播放条目。
//
// 只读 backend 的 `playQueue`（id / name / current），不碰直链。
// 用 ListView 而不是 Repeater：音乐文件夹可能有上千首，不能一次建完所有行。
Popup {
    id: panel

    property var backend: null
    readonly property int rowHeight: 34
    property int maxPopupHeight: 420

    width: 360
    height: Math.min(maxPopupHeight,
                     Math.max(rowHeight, (backend ? backend.playQueue.length : 0) * rowHeight) + 12)
    padding: 6
    modal: false
    focus: true
    closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside

    // 和轨道菜单同一套固定深色表面，不跟桌面主题。
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

    contentItem: ListView {
        id: list

        clip: true
        model: panel.backend ? panel.backend.playQueue : []
        boundsBehavior: Flickable.StopAtBounds
        ScrollBar.vertical: KScrollBar {}

        delegate: PlaybackPopoverRow {
            width: list.width - 10
            // 解析不出集数就显示队列位置 `#N`，明确它只是顺序、不是集数。
            badge: modelData.episode && modelData.episode.length > 0
                   ? modelData.episode : ("#" + (index + 1))
            position: index + 1
            // 队列位置始终显示：同一集有多个版本时，集数会重复，位置才是唯一顺序。
            showPosition: true
            text: modelData.name
            selected: panel.backend !== null && index === panel.backend.playQueueIndex
            onClicked: {
                if (panel.backend !== null)
                    panel.backend.playQueueItem(index)
                panel.close()
            }
        }
    }

    /// 右对齐贴在锚点按钮上方 8px 处，打开后把当前集滚到中间。
    function openAbove(anchor) {
        var origin = anchor.mapToItem(null, 0, 0)
        x = Math.round(origin.x + anchor.width - width)
        y = Math.round(origin.y - height - 8)
        open()
        Qt.callLater(function () {
            if (panel.backend !== null && panel.backend.playQueueIndex >= 0)
                list.positionViewAtIndex(panel.backend.playQueueIndex, ListView.Center)
        })
    }
}
