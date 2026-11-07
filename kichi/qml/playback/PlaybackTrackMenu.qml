import QtQuick
import QtQuick.Controls

// 播放窗口的轨道菜单：音轨或字幕轨共用。
//
// 深色自绘浮层，不用系统 Menu（浅色底 + 系统复选框 / 滚动条）。
// 菜单内容不预取：每次打开现读 `backend.embedTracks()`，这样 mpv 原生键位
// 切过轨、或播放中补挂了字幕，勾选状态和列表都是最新的。
//
// 轨道可能有几十条：「关闭字幕」固定在头部、「字幕样式…」固定在尾部，
// 只有中间的轨道列表滚动；打开时自动滚到当前选中的轨道。
PlaybackPopover {
    id: menu

    property var backend: null
    /// "audio" / "sub"
    property string kind: "audio"
    /// 可滚动的轨道行：{ id, label, selected, empty }
    property var rows: []
    /// 头部「关闭字幕」是否打勾（没有选中任何字幕轨时）
    property bool closeSelected: false

    signal trackSelected(string kind, int id, string label)
    signal styleRequested()

    readonly property int rowHeight: 34
    readonly property int dividerHeight: kind === "sub" ? 9 : 0
    readonly property int styleRowHeight: kind === "sub" ? rowHeight : 0
    /// 只有真实字幕轨才显示固定头；空状态只留占位行
    readonly property bool hasCloseRow: kind === "sub" && rows.length > 0
                                      && rows[0].empty !== true

    popupWidth: 260

    function openAt(anchor) {
        rebuild()
        // 行高显式定，直接算总高：模型刚换过时 Column 的 implicitHeight 还没刷新。
        bodyHeight = (hasCloseRow ? rowHeight : 0) + rows.length * rowHeight
                   + dividerHeight + styleRowHeight
        openAbove(anchor)
        Qt.callLater(function () { menu.ensureRowVisible(menu.selectedRowIndex(), menu.rowHeight) })
    }

    function selectedRowIndex() {
        for (var i = 0; i < menu.rows.length; ++i) {
            if (menu.rows[i].selected === true)
                return i
        }
        return 0
    }

    function rebuild() {
        var list = []
        var tracks = menu.backend ? menu.backend.embedTracks() : []
        var anySelected = false
        for (var i = 0; i < tracks.length; ++i) {
            if (tracks[i].kind !== menu.kind)
                continue
            list.push({
                "id": tracks[i].id,
                "label": tracks[i].label,
                "selected": tracks[i].selected === true,
                "empty": false
            })
            if (tracks[i].selected)
                anySelected = true
        }
        if (list.length === 0) {
            list.push({
                "id": -2,
                "label": menu.kind === "sub" ? qsTr("此视频没有字幕轨道")
                                             : qsTr("此视频没有音轨"),
                "selected": false,
                "empty": true
            })
        }
        menu.rows = list
        menu.closeSelected = menu.kind === "sub" && list[0].empty !== true && !anySelected
    }

    headerItems: PlaybackPopoverRow {
        width: parent.width
        visible: menu.hasCloseRow
        text: qsTr("关闭字幕")
        selected: menu.closeSelected
        onClicked: {
            menu.trackSelected("sub", -1, qsTr("关闭字幕"))
            menu.close()
        }
    }

    // 可滚动区：只放轨道行。
    Repeater {
        model: menu.rows

        delegate: PlaybackPopoverRow {
            width: parent.width
            text: modelData.label
            selected: modelData.selected === true
            interactive: modelData.empty !== true
            onClicked: {
                menu.trackSelected(menu.kind, modelData.id, modelData.label)
                menu.close()
            }
        }
    }

    footerItems: Item {
        width: parent.width
        height: visible ? menu.dividerHeight + menu.styleRowHeight : 0
        visible: menu.kind === "sub"

        Rectangle {
            anchors.top: parent.top
            anchors.topMargin: 4
            anchors.horizontalCenter: parent.horizontalCenter
            width: parent.width - 16
            height: 1
            color: "#1FFFFFFF"
        }

        PlaybackPopoverRow {
            anchors.top: parent.top
            anchors.topMargin: menu.dividerHeight
            width: parent.width
            text: qsTr("字幕样式…")
            trailingIcon: "chevron-right"
            onClicked: menu.styleRequested()
        }
    }
}
