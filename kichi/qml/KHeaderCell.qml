import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 表头文字格：统一 muted + DemiBold + 垂直居中。
//
// 列宽走 Layout：fillWidth 吃掉剩余宽度；fixedWidth 用 cellWidth
// （响应式隐藏时调用方把 cellWidth 一起绑到 0）。水平对齐仍由调用方设置。
Label {
    id: cell

    property bool fillWidth: false
    property bool fixedWidth: false
    property int cellWidth: 0
    /// -1 表示跟随内容宽度
    property int minimumCellWidth: -1
    property bool elideText: false

    Layout.fillWidth: cell.fillWidth
    Layout.preferredWidth: cell.fixedWidth ? cell.cellWidth : cell.implicitWidth
    Layout.minimumWidth: cell.minimumCellWidth

    color: theme.textMuted
    font.pixelSize: theme.fontMeta
    font.weight: Font.DemiBold
    verticalAlignment: Text.AlignVCenter
    elide: cell.elideText ? Text.ElideRight : Text.ElideNone

    KTheme { id: theme }
}
