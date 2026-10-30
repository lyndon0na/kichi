import QtQuick

// 传输任务表格的唯一密度来源：表头、父行、子行共用。
//
// 行高 48 / 表头 34 与文件、分享、离线任务表格一致。列宽全部固定，
// 名称列吃剩余空间；窄窗口只隐藏速度 / 大小，不改其它列的横向位置。
Item {
    id: density

    visible: false

    property real densityScale: 1.0

    readonly property int rowHeight: Math.round(48 * densityScale)
    readonly property int headerRowHeight: Math.round(34 * densityScale)

    // 箭头也放在图标列里：父行显示「› + 文件夹」，顶层普通文件也空出箭头槽，
    // 让类型图标落在同一竖线；子行沿用现有缩进。
    readonly property int iconColumnWidth: Math.round(38 * densityScale)
    readonly property int statusColumnWidth: Math.round(170 * densityScale)
    readonly property int progressColumnWidth: Math.round(150 * densityScale)
    readonly property int sizeColumnWidth: Math.round(112 * densityScale)
    readonly property int rateColumnWidth: Math.round(170 * densityScale)
    readonly property int actionColumnWidth: Math.round(144 * densityScale)

    readonly property int rowPadding: 12
    readonly property int columnSpacing: 10
    /// 每一级子行的额外缩进；只挤压名称列，不移动右侧固定列。
    readonly property int indentWidth: Math.round(18 * densityScale)
}
