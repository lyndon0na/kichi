import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 文件页表头：列名 + 点列排序。
//
// 列宽由调用方（FilesPage）同时交给本组件和 FileRow，两边共用一套数值：
// 表头那行和每一行都按同一组宽度摆位，改一处两边一起动。
// 排序直接调 backend.setSorting：本地重排、不重新拉网，与设置页是同一份状态，
// 所以从设置页改了排序，回来箭头也跟着变。
Item {
    id: header

    property var backend: null
    // 列宽 / 水平内边距与 FileRow 一致：实际数值由 FilesPage 同时下发，
    // 这里的默认值只是兜底（和 FileRow 的默认值保持相同）。
    property int iconColumnWidth: 24
    property int sizeColumnWidth: 100
    property int modifiedColumnWidth: 130
    property int contentPadding: 8
    property int columnSpacing: 10

    // 26 的行高 + 底部 1px 分隔线
    implicitHeight: 27

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

    // 列名后面挂方向箭头（只在当前排序列出现）
    function label(base, field) {
        if (!header.backend || header.backend.sortField !== field)
            return base
        return base + (header.backend.sortAscending ? " ▲" : " ▼")
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        RowLayout {
            Layout.fillWidth: true
            Layout.leftMargin: header.contentPadding
            Layout.rightMargin: header.contentPadding
            Layout.preferredHeight: 26
            spacing: header.columnSpacing

            // 图标列占位：让「名称」和行的文件名左边缘对齐
            Item {
                Layout.preferredWidth: header.iconColumnWidth
                Layout.preferredHeight: 1
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
                    color: palette.placeholderText
                    font.pixelSize: 12
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
                    text: header.label(qsTr("大小"), sizeCell.field)
                    color: palette.placeholderText
                    font.pixelSize: 12
                    verticalAlignment: Text.AlignVCenter
                    horizontalAlignment: Text.AlignRight
                    elide: Text.ElideRight
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
                    text: header.label(qsTr("修改时间"), modifiedCell.field)
                    color: palette.placeholderText
                    font.pixelSize: 12
                    verticalAlignment: Text.AlignVCenter
                    horizontalAlignment: Text.AlignRight
                    elide: Text.ElideRight
                }

                TapHandler {
                    onTapped: header.toggle(modifiedCell.field)
                }

            }
        }

        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 1
            color: palette.mid
        }
    }
}
