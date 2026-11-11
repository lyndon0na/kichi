import QtQuick
import QtQuick.Controls

// 搜索 / 文件页共用的筛选入口：漏斗按钮 + 三组可多选的勾选菜单。
//
// 筛选状态只存在后端（searchFilter* 属性）里，两个入口看到的是同一份条件。
// 同组多选取并集；每组第一项「全部 / 不限」是「该组不限制」的快捷勾选，
// 勾上它等于清空该组。勾选任何一项都不会关闭菜单，方便连着勾几项；
// 点「清除筛选」才收起。
//
// 面板内三列并排：类型 | 修改时间 + 大小 | 排序，高度只取最高的一列，
// 不再把十几项纵向堆成一长条。勾选任何一项都不会关闭菜单，方便连着勾几项；
// 点「清除筛选」才收起。
//
// 排序只有三个列名，不再单列「升序 / 降序」：当前列名后带 ↑ / ↓，
// 点当前列切换升降序，点其它列换成那一列（用该列的常用方向）。
// 排序不是筛选条件，所以排序行**不打勾**（勾只表示筛选条件），
// 方向箭头就是它自己的选中标记；也不计入按钮上的数字，走的是后端既有的
// setSorting。对目录列表和搜索结果都生效
// （搜索结果仍是匹配质量优先，只在同一档内换列）。
KButton {
    id: root

    property var backend: null

    // 筛选变化只改 Rust 状态；由使用方决定怎么重跑当前关键词。
    signal filterChanged()

    // 旧测试替身可能没有筛选属性，按「没有筛选」处理。
    readonly property int activeFilterCount: backend !== null
                                             && typeof backend.searchFilterActiveCount !== "undefined"
                                             ? backend.searchFilterActiveCount : 0
    readonly property var currentTypes: backend !== null
                                        && typeof backend.searchFilterTypes !== "undefined"
                                        ? (backend.searchFilterTypes || []) : []
    readonly property var currentModifieds: backend !== null
                                            && typeof backend.searchFilterModifieds !== "undefined"
                                            ? (backend.searchFilterModifieds || []) : []
    readonly property var currentSizes: backend !== null
                                        && typeof backend.searchFilterSizes !== "undefined"
                                        ? (backend.searchFilterSizes || []) : []
    // 排序是视图策略，不进筛选条件、也不计入按钮上的数字。
    readonly property string currentSortField: backend !== null
                                               && typeof backend.sortField !== "undefined"
                                               ? (backend.sortField || "name") : "name"
    readonly property bool currentSortAscending: backend !== null
                                                  && typeof backend.sortAscending !== "undefined"
                                                  ? backend.sortAscending === true : true

    readonly property var typeOptions: [
        { value: "all", label: qsTr("全部"), indent: 0 },
        { value: "folder", label: qsTr("文件夹"), indent: 0 },
        { value: "file", label: qsTr("文件"), indent: 0 },
        { value: "video", label: qsTr("视频"), indent: 1 },
        { value: "audio", label: qsTr("音频"), indent: 1 },
        { value: "image", label: qsTr("图片"), indent: 1 },
        { value: "document", label: qsTr("文档"), indent: 1 },
        { value: "archive", label: qsTr("压缩包"), indent: 1 },
        { value: "other", label: qsTr("其他"), indent: 1 }
    ]
    readonly property var modifiedOptions: [
        { value: "any", label: qsTr("不限") },
        { value: "7d", label: qsTr("最近 7 天") },
        { value: "30d", label: qsTr("最近 30 天") },
        { value: "1y", label: qsTr("最近 1 年") }
    ]
    readonly property var sizeOptions: [
        { value: "any", label: qsTr("不限") },
        { value: "lt10m", label: qsTr("<10 MB") },
        { value: "10m-100m", label: qsTr("10–100 MB") },
        { value: "100m-1g", label: qsTr("100 MB–1 GB") },
        { value: "gt1g", label: qsTr(">1 GB") }
    ]
    readonly property var sortFieldOptions: [
        { value: "name", label: qsTr("名称") },
        { value: "size", label: qsTr("大小") },
        { value: "modified", label: qsTr("修改时间") }
    ]

    /// 某一项当前是否打勾：`all` / `any` 表示「该组没有任何限制」。
    function isChecked(group, value) {
        const list = group === "type" ? currentTypes
                   : group === "modified" ? currentModifieds : currentSizes
        if (value === "all" || value === "any")
            return list.length === 0
        return list.indexOf(value) >= 0
    }

    function applyFilter(types, modifieds, sizes) {
        if (!root.backend)
            return
        root.backend.setSearchFilter(types, modifieds, sizes)
        root.filterChanged()
    }

    /// 多选：点一项加入 / 移出该组；点「全部 / 不限」清空该组。
    function toggle(group, value) {
        const types = currentTypes.slice()
        const modifieds = currentModifieds.slice()
        const sizes = currentSizes.slice()
        const list = group === "type" ? types
                   : group === "modified" ? modifieds : sizes
        if (value === "all" || value === "any") {
            list.length = 0
        } else {
            const index = list.indexOf(value)
            if (index >= 0)
                list.splice(index, 1)
            else
                list.push(value)
        }
        applyFilter(types, modifieds, sizes)
    }

    function clearFilters() {
        if (!root.backend)
            return
        root.backend.clearSearchFilters()
        root.filterChanged()
    }

    // 换排序列时用该列的常用方向：名称 A→Z，大小 / 时间把大的、新的放前面。
    // 与 FileSortMenu / FileListHeader.defaultAscending 是同一条规则，三处要一起改。
    function defaultAscending(field) {
        return field === "name"
    }

    function applySortField(field) {
        if (!root.backend)
            return
        root.backend.setSorting(field, root.defaultAscending(field))
    }

    /// 排序只有一组：点已选中的列 = 切换升降序，点别的列 = 换列并用该列常用方向。
    /// 当前方向就写在列名后面（↑ / ↓），不再单独占「升序 / 降序」两行。
    function applySortToggle(field) {
        if (!root.backend)
            return
        if (field === root.currentSortField) {
            root.backend.setSorting(field, !root.currentSortAscending)
        } else {
            root.applySortField(field)
        }
    }

    objectName: "searchFilterButton"
    text: root.activeFilterCount > 0
          ? qsTr("筛选 %1").arg(root.activeFilterCount)
          : qsTr("筛选")
    iconName: "filter"
    iconSize: 14
    variant: "ghost"
    // 有激活条件时淡强调底 + 强调色文字，和「当前在此页」同一套语言。
    pageActive: root.activeFilterCount > 0
    implicitHeight: 28
    onClicked: filterMenu.visible ? filterMenu.close() : filterMenu.openBelow(root)

    // 分组菜单：同组可多选，勾选态直接读后端属性。
    // 三组筛选与排序并排成三列，面板高度只取最高的一列，小窗口也能完整看到。
    // 父项固定成 Overlay.overlay，坐标用窗口坐标（坑 10），宽度只写在 Menu 上（坑 9）。
    Menu {
        id: filterMenu

        objectName: "searchFilterMenu"
        parent: Overlay.overlay
        // 三列宽度 + 列间距 + 左右内边距：150 + 140 + 132 + 2×12 + 2×4
        implicitWidth: 454
        padding: 4

        readonly property FontMetrics metrics: FontMetrics {
            font: filterMenu.font
        }
        readonly property int rowHeight: Math.max(24, metrics.height + 10)

        function openBelow(item) {
            const point = item.mapToItem(null, 0, item.height + 2)
            // 面板比较宽：右边放不下就往左收，别把排序那一列顶出窗口
            const overlayWidth = filterMenu.parent ? filterMenu.parent.width : 0
            const maxX = overlayWidth - filterMenu.width - 8
            const x = maxX > 0 ? Math.min(point.x, maxX) : point.x
            filterMenu.popup(Qt.point(Math.max(8, x), point.y))
        }

        MenuItem {
            height: filterMenu.rowHeight
            topPadding: 0
            bottomPadding: 0
            text: qsTr("筛选")
            enabled: false
        }

        MenuSeparator {
            topPadding: 0
            bottomPadding: 0
        }

        // Row / Column 里的 MenuItem 不归 Menu 自己管：点勾选不会把面板收起。
        Row {
            spacing: 12

            // 类型：文件夹与文件平铺，文件子类型缩进一档
            Column {
                id: typeColumn

                width: 150
                spacing: 0

                MenuItem {
                    width: typeColumn.width
                    height: filterMenu.rowHeight
                    topPadding: 0
                    bottomPadding: 0
                    text: qsTr("类型")
                    enabled: false
                    font.pixelSize: 11
                    font.weight: Font.DemiBold
                }

                Repeater {
                    objectName: "filterTypeRepeater"
                    model: root.typeOptions

                    delegate: MenuItem {
                        objectName: "filterType-" + modelData.value
                        width: typeColumn.width
                        height: filterMenu.rowHeight
                        topPadding: 0
                        bottomPadding: 0
                        text: modelData.label
                        // 文件子类型缩进一档，表明它们属于「文件」。
                        leftPadding: 12 + modelData.indent * 16
                        checkable: true
                        // 不在 Menu 自己的菜单项列表里，靠自身 hover 点亮
                        hoverEnabled: true
                        highlighted: hovered
                        checked: root.isChecked("type", modelData.value)
                        onTriggered: root.toggle("type", modelData.value)
                    }
                }
            }

            // 修改时间 + 大小
            Column {
                id: filterColumn

                width: 140
                spacing: 0

                MenuItem {
                    width: filterColumn.width
                    height: filterMenu.rowHeight
                    topPadding: 0
                    bottomPadding: 0
                    text: qsTr("修改时间")
                    enabled: false
                    font.pixelSize: 11
                    font.weight: Font.DemiBold
                }

                Repeater {
                    objectName: "filterModifiedRepeater"
                    model: root.modifiedOptions

                    delegate: MenuItem {
                        objectName: "filterModified-" + modelData.value
                        width: filterColumn.width
                        height: filterMenu.rowHeight
                        topPadding: 0
                        bottomPadding: 0
                        text: modelData.label
                        checkable: true
                        // 不在 Menu 自己的菜单项列表里，靠自身 hover 点亮
                        hoverEnabled: true
                        highlighted: hovered
                        checked: root.isChecked("modified", modelData.value)
                        onTriggered: root.toggle("modified", modelData.value)
                    }
                }

                MenuSeparator {
                    width: filterColumn.width
                    topPadding: 0
                    bottomPadding: 0
                }

                MenuItem {
                    width: filterColumn.width
                    height: filterMenu.rowHeight
                    topPadding: 0
                    bottomPadding: 0
                    text: qsTr("大小")
                    enabled: false
                    font.pixelSize: 11
                    font.weight: Font.DemiBold
                }

                Repeater {
                    objectName: "filterSizeRepeater"
                    model: root.sizeOptions

                    delegate: MenuItem {
                        objectName: "filterSize-" + modelData.value
                        width: filterColumn.width
                        height: filterMenu.rowHeight
                        topPadding: 0
                        bottomPadding: 0
                        text: modelData.label
                        checkable: true
                        // 不在 Menu 自己的菜单项列表里，靠自身 hover 点亮
                        hoverEnabled: true
                        highlighted: hovered
                        checked: root.isChecked("size", modelData.value)
                        onTriggered: root.toggle("size", modelData.value)
                    }
                }
            }

            // 排序：列名后面直接带方向箭头，点已选中的列切换升降。
            // 不打勾：勾在这块面板里只表示筛选条件，排序是视图策略。
            Column {
                id: sortColumn

                width: 132
                spacing: 0

                MenuItem {
                    width: sortColumn.width
                    height: filterMenu.rowHeight
                    topPadding: 0
                    bottomPadding: 0
                    text: qsTr("排序")
                    enabled: false
                    font.pixelSize: 11
                    font.weight: Font.DemiBold
                }

                Repeater {
                    objectName: "filterSortFieldRepeater"
                    model: root.sortFieldOptions

                    delegate: MenuItem {
                        objectName: "filterSort-" + modelData.value
                        readonly property bool activeSort: root.currentSortField === modelData.value

                        width: sortColumn.width
                        height: filterMenu.rowHeight
                        topPadding: 0
                        bottomPadding: 0
                        // 当前列名后带 ↑ / ↓，一眼看出按什么、往哪个方向排
                        text: modelData.label
                              + (activeSort ? (root.currentSortAscending ? "  ↑" : "  ↓") : "")
                        // 不在 Menu 自己的菜单项列表里，靠自身 hover 点亮
                        hoverEnabled: true
                        highlighted: hovered
                        onTriggered: root.applySortToggle(modelData.value)
                    }
                }
            }
        }

        MenuSeparator {
            visible: root.activeFilterCount > 0
            topPadding: 0
            bottomPadding: 0
        }

        MenuItem {
            objectName: "searchFilterClearItem"
            visible: root.activeFilterCount > 0
            height: filterMenu.rowHeight
            topPadding: 0
            bottomPadding: 0
            text: qsTr("清除筛选")
            onTriggered: root.clearFilters()
        }
    }
}
