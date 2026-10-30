import QtQuick

// 文件列表的密度：行高 / 字号 / 图标 / 列宽。
//
// 表头（FileListHeader）、每一行（FileRow）和搜索结果（SearchResults）
// 都从这里取同一组数值——只改这一处，三边一起变，不会出现
// 「表头 100px、行里 90px」的错位（见 docs/pitfalls.md 第 8 条）。
//
// 基准取应用默认字体（本机 KDE 配的是 Noto Sans 10pt）：字体按点值配置时
// font.pixelSize 为 -1，退回 13，正是 10pt 在 96dpi 下的实际像素。
// 要整体放大 / 缩小只改 densityScale。
//
// 字号只有三档：文件名 14、次要信息 12、图标 18。
// 原来文件名 16 而其它全是 12，屏幕上每样东西差不多大，看不出主次。
// 行高 48 / 表头 34：比旧版（44 / 32）多一点纵向呼吸，又仍然一屏能看十几行；
// 行内左右内边距统一到 12，和页头的左边距对齐。
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

    readonly property int namePixelSize: Math.round(baseFontSize * 1.08 * densityScale)
    readonly property int metaPixelSize: Math.round(baseFontSize * 0.92 * densityScale)
    readonly property int iconPixelSize: Math.round(baseFontSize * 1.38 * densityScale)

    readonly property int rowHeight: Math.round(baseFontSize * 3.7 * densityScale)
    readonly property int headerRowHeight: Math.round(baseFontSize * 2.6 * densityScale)

    // 搜索结果仍用紧凑图标列；目录 / 回收站的首列是固定缩略图槽，
    // 没有可用缩略图时在同一个槽里回退为文件图标。
    readonly property int iconColumnWidth: Math.round(22 * densityScale)
    readonly property int thumbnailSlotWidth: Math.round(36 * densityScale)
    readonly property int thumbnailSlotHeight: Math.round(24 * densityScale)
    readonly property int sizeColumnWidth: Math.round(96 * densityScale)
    readonly property int modifiedColumnWidth: Math.round(150 * densityScale)

    // 搜索结果的行是两行（名字 + 所在文件夹），行高和列宽单独一档
    readonly property int resultRowHeight: Math.round(rowHeight * 1.15)
    readonly property int resultSizeColumnWidth: Math.round(96 * densityScale)
    readonly property int resultModifiedColumnWidth: Math.round(150 * densityScale)

    // 水平内边距与列间距：FileRow 显式用它，表头也用同一个值，
    // 不跟控件风格的默认 padding 走（风格默认值会随风格变）。
    // 统一到 12：8 太挤、16 太散，且两者都跟「行高 44」不成比例。
    readonly property int rowPadding: 12
    readonly property int columnSpacing: 12

    // —— 网格视图 ——
    // 缩略图保持 3:2；列数由 FilesPage 按可用宽度计算，委托只负责自己的内容。
    readonly property int gridMinItemWidth: Math.round(160 * densityScale)
    readonly property int gridColumnGap: 14
    readonly property int gridRowGap: 18
    readonly property int gridCardPadding: 5
    readonly property int gridCardSpacing: 7
    readonly property int gridTextSpacing: 2
    readonly property int gridIconPixelSize: Math.round(28 * densityScale)
    readonly property int gridVideoBadgeSize: Math.round(18 * densityScale)
    readonly property int gridNameLineHeight: Math.round(namePixelSize * 1.35)
    readonly property int gridMetaLineHeight: Math.round(metaPixelSize * 1.3)

    /// 一个网格委托的完整高度（不含行与行之间的 gridRowGap）。
    function gridItemHeight(itemWidth) {
        const thumbWidth = Math.max(1, itemWidth - gridCardPadding * 2)
        const thumbHeight = Math.max(1, Math.ceil(thumbWidth * 2 / 3))
        return gridCardPadding * 2 + thumbHeight + gridCardSpacing
                + gridNameLineHeight + gridTextSpacing + gridMetaLineHeight
    }
}
