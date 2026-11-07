import QtQuick
import QtQuick.Layouts
import "../core"

// 静态表头骨架：固定行高 + 左右内边距 + 列间距 + 底部 1px 分隔线。
//
// 列内容由调用方写在默认插槽里（真实 QML 子项，列宽绑定保持存活）。
// 可排序的文件列表表头（FileListHeader）结构不同，不用本组件。
Item {
    id: header

    property int headerRowHeight: 34
    property int contentPadding: 12
    property int columnSpacing: 12

    implicitHeight: header.headerRowHeight
    default property alias content: layout.data

    KTheme { id: theme }

    RowLayout {
        id: layout
        anchors.fill: parent
        anchors.leftMargin: header.contentPadding
        anchors.rightMargin: header.contentPadding
        spacing: header.columnSpacing
    }

    Rectangle {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        height: 1
        color: theme.hairline
    }
}
