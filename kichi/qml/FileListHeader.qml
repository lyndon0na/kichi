import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 文件页表头：列名 + 点列排序。
//
// 列宽由调用方（FilesPage）同时交给本组件和 FileRow，两边共用一套数值：
// 表头那行和每一行都按同一组宽度摆位，改一处两边一起动。
// 排序直接调 backend.setSorting：本地重排、不重新拉网。
Item {
    id: header

    property var backend: null
    // 列宽 / 水平内边距与 FileRow 一致：实际数值由 FilesPage 同时下发，
    // 这里的默认值只是兜底（和 FileRow 的默认值保持相同）。
    property int thumbnailSlotWidth: 36
    property int sizeColumnWidth: 96
    property int modifiedColumnWidth: 150
    property int contentPadding: 12
    property int columnSpacing: 12
    // 表头行高与字号：和列表行同一套密度，由 FilesPage 下发
    property int headerRowHeight: 32
    property int headerPixelSize: 12

    // 表头本身铺满整个列表区（含滚动条车道）：行宽已减去车道，
    // 右侧再补一个车道宽度，列才会和行对齐
    UiMetrics {
        id: metrics
    }

    KTheme { id: theme }

    // 分隔线覆盖在表头底部，不额外增加布局高度。
    implicitHeight: header.headerRowHeight

    // 换列时的常用方向：名称从 A 到 Z，大小和时间都把大的 / 新的放前面
    function defaultAscending(field) {
        return field === "name"
    }

    // 点表头：同一列切换方向；换列用该列的常用方向
    function toggle(field) {
        if (!header.backend)
            return
        var sameField = header.backend.sortField === field
        var ascending = sameField ? !header.backend.sortAscending
                                  : header.defaultAscending(field)
        header.backend.setSorting(field, ascending)
    }

    // 列名。左对齐的「名称」把箭头直接接在文字后面（左边缘不动，不会跳）；
    // 右对齐的两列由 caret() 单独画在固定槽位里，否则切换排序时整段文字会左右平移。
    function label(base, field) {
        if (field !== "name")
            return base
        var mark = header.caret(field)
        return mark.length > 0 ? base + " " + mark : base
    }

    // 排序箭头：这一列不是当前排序列时返回空串（槽位仍然占着）
    function caret(field) {
        if (!header.backend || header.backend.sortField !== field)
            return ""
        return header.backend.sortAscending ? "▲" : "▼"
    }

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: header.contentPadding
        // 行宽比表头窄一个滚动条车道：右内边距同步 +10，
        // 否则「修改时间」列头和行里的时间差 10px
        anchors.rightMargin: header.contentPadding + metrics.scrollBarSlot
        spacing: header.columnSpacing

        // 图标列占位：让「名称」和行的文件名左边缘对齐
        Item {
            Layout.preferredWidth: header.thumbnailSlotWidth
            Layout.fillHeight: true
        }

        // —— 名称 ——
        Item {
            id: nameCell
            readonly property string field: "name"

            Layout.fillWidth: true
            Layout.fillHeight: true

            HoverHandler {
                id: nameHover
                cursorShape: Qt.PointingHandCursor
            }

            Label {
                anchors.fill: parent
                text: header.label(qsTr("名称"), nameCell.field)
                color: theme.textMuted
                font.pixelSize: header.headerPixelSize
                font.weight: Font.DemiBold
                verticalAlignment: Text.AlignVCenter
                horizontalAlignment: Text.AlignLeft
                elide: Text.ElideRight
            }

            TapHandler {
                onTapped: header.toggle(nameCell.field)
            }

        }

        // —— 大小 ——
        Item {
            id: sizeCell
            readonly property string field: "size"

            Layout.preferredWidth: header.sizeColumnWidth
            Layout.fillHeight: true

            HoverHandler {
                id: sizeHover
                cursorShape: Qt.PointingHandCursor
            }

            Label {
                anchors.fill: parent
                // 右侧留出固定的箭头槽位，排序切换时表头文字不左右跳
                anchors.rightMargin: 14
                text: header.label(qsTr("大小"), sizeCell.field)
                color: theme.textMuted
                font.pixelSize: header.headerPixelSize
                font.weight: Font.DemiBold
                verticalAlignment: Text.AlignVCenter
                horizontalAlignment: Text.AlignRight
                elide: Text.ElideRight
            }

            Label {
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                width: 14
                text: header.caret(sizeCell.field)
                color: header.backend && header.backend.sortField === sizeCell.field
                       ? theme.accent : theme.textMuted
                font.pixelSize: header.headerPixelSize
                font.weight: Font.DemiBold
                horizontalAlignment: Text.AlignRight
            }

            TapHandler {
                onTapped: header.toggle(sizeCell.field)
            }

        }

        // —— 修改时间 ——
        Item {
            id: modifiedCell
            readonly property string field: "modified"

            Layout.preferredWidth: header.modifiedColumnWidth
            Layout.fillHeight: true

            HoverHandler {
                id: modifiedHover
                cursorShape: Qt.PointingHandCursor
            }

            Label {
                anchors.fill: parent
                anchors.rightMargin: 14
                text: header.label(qsTr("修改时间"), modifiedCell.field)
                color: theme.textMuted
                font.pixelSize: header.headerPixelSize
                font.weight: Font.DemiBold
                verticalAlignment: Text.AlignVCenter
                horizontalAlignment: Text.AlignRight
                elide: Text.ElideRight
            }

            Label {
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                width: 14
                text: header.caret(modifiedCell.field)
                color: header.backend && header.backend.sortField === modifiedCell.field
                       ? theme.accent : theme.textMuted
                font.pixelSize: header.headerPixelSize
                font.weight: Font.DemiBold
                horizontalAlignment: Text.AlignRight
            }

            TapHandler {
                onTapped: header.toggle(modifiedCell.field)
            }

        }
    }

    Rectangle {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        height: 1
        color: theme.hairline
    }
}
