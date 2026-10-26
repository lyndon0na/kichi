import QtQuick

// 文件列表的密度：行高 / 字号 / 图标 / 列宽。
//
// 表头（FileListHeader）、每一行（FileRow）和搜索结果（SearchResults）
// 都从这里取同一组数值——只改这一处，三边一起变，不会出现
// 「表头 100px、行里 90px」的错位（见 docs/pitfalls.md 第 8 条）。
//
// 基准取应用默认字体（本机 KDE 配的是 Noto Sans 10pt）：字体按点值配置时
// font.pixelSize 为 -1，退回 13，正是 10pt 在 96dpi 下的实际像素。
// 要整体放大 / 缩小只改 densityScale；1.0 = 2026-10-26 用户选定的「宽松」档。
//
// 本身不可见、不占位：它只是一个装着数值的组件，顺带带一个隐藏 Text
// 用来读应用默认字体。
Item {
    id: density

    visible: false

    // 只为了读应用默认字体，不参与布局（父项不可见，它也不会画出来）
    Text {
        id: fontProbe
        visible: false
    }

    property real densityScale: 1.0
    readonly property real baseFontSize: fontProbe.font.pixelSize > 0
                                         ? fontProbe.font.pixelSize : 13

    readonly property int namePixelSize: Math.round(baseFontSize * 1.23 * densityScale)
    readonly property int metaPixelSize: Math.round(baseFontSize * 1.0 * densityScale)
    readonly property int iconPixelSize: Math.round(baseFontSize * 1.85 * densityScale)

    readonly property int rowHeight: Math.round(baseFontSize * 4.0 * densityScale)
    readonly property int headerRowHeight: Math.round(baseFontSize * 2.77 * densityScale)

    // 图标列要比图标本身宽：24px 的 emoji 字宽超过 24，列宽不够会压到文件名
    readonly property int iconColumnWidth: Math.round(30 * densityScale)
    readonly property int sizeColumnWidth: Math.round(126 * densityScale)
    readonly property int modifiedColumnWidth: Math.round(165 * densityScale)

    // 搜索结果的行是两行（名字 + 所在文件夹），行高和列宽单独一档
    readonly property int resultRowHeight: Math.round(rowHeight * 1.15)
    readonly property int resultSizeColumnWidth: Math.round(110 * densityScale)
    readonly property int resultModifiedColumnWidth: Math.round(150 * densityScale)

    // 水平内边距与列间距：FileRow 显式用它，表头也用同一个值，
    // 不跟控件风格的默认 padding 走（风格默认值会随风格变）。
    readonly property int rowPadding: 8
    readonly property int columnSpacing: 10
}
