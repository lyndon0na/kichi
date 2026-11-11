import QtQuick
import QtTest
import "../components"

// 文件页搜索状态行的筛选入口。
//
// 锁住六件事：
// ① 漏斗按钮的文案带激活条件数；
// ② 状态行显示筛选摘要；
// ③ 菜单按三组列出、已勾选项打勾；
// ④ 点某一项把三组线名一起交给后端（不会把其它两组清掉），并发信号让文件页重跑；
// ⑤ 同组多选取并集：再点一次取消该项，勾「全部 / 不限」清空该组；菜单保持打开；
// ⑥ 有筛选时才出现「清除筛选」；
// ⑦ 排序只有三个列名：当前列带 ↑ / ↓，点当前列切换升降序，点别的列换列；
//    选排序列 / 方向都走 setSorting，且不收起菜单。
//
// 运行：
//   QT_QPA_PLATFORM=offscreen /usr/lib64/qt6/bin/qmltestrunner \
//       -input kichi/qml/tests/tst_search_status_bar.qml
TestCase {
    name: "SearchStatusBar"
    when: windowShown
    width: 900
    // 三列并排后普通窗口高度就够点到底部的排序项
    height: 800

    // 替代 Kichi 桥接对象：只提供状态行用到的属性 / 方法。
    QtObject {
        id: fakeBackend
        property bool searchActive: true
        property bool searchSettled: true
        property string searchSource: "index"
        property string searchStatus: "本地索引命中 3 条"
        property bool searchTruncated: false
        property int searchFilterActiveCount: 0
        property string searchFilterSummary: ""
        property var searchFilterTypes: []
        property var searchFilterModifieds: []
        property var searchFilterSizes: []
        property int setCalls: 0
        property var lastTypes: []
        property var lastModifieds: []
        property var lastSizes: []
        property int clearCalls: 0
        property string sortField: "name"
        property bool sortAscending: true
        property int sortCalls: 0
        property string lastSortField: ""
        property bool lastSortAscending: true

        function setSearchFilter(types, modifieds, sizes) {
            setCalls += 1
            lastTypes = types
            lastModifieds = modifieds
            lastSizes = sizes
            searchFilterTypes = types
            searchFilterModifieds = modifieds
            searchFilterSizes = sizes
        }

        function clearSearchFilters() {
            clearCalls += 1
            searchFilterActiveCount = 0
            searchFilterSummary = ""
            searchFilterTypes = []
            searchFilterModifieds = []
            searchFilterSizes = []
        }

        function setSorting(field, ascending) {
            sortCalls += 1
            lastSortField = field
            lastSortAscending = ascending
            sortField = field
            sortAscending = ascending
        }

        function stopSearch() {}
    }

    Component {
        id: barComponent
        SearchStatusBar {
            width: 700
            backend: fakeBackend
        }
    }

    property var bar: null
    property var menu: null
    property int filterRequests: 0

    function resetBackend() {
        fakeBackend.searchFilterActiveCount = 0
        fakeBackend.searchFilterSummary = ""
        fakeBackend.searchFilterTypes = []
        fakeBackend.searchFilterModifieds = []
        fakeBackend.searchFilterSizes = []
        fakeBackend.setCalls = 0
        fakeBackend.lastTypes = []
        fakeBackend.lastModifieds = []
        fakeBackend.lastSizes = []
        fakeBackend.clearCalls = 0
        fakeBackend.sortField = "name"
        fakeBackend.sortAscending = true
        fakeBackend.sortCalls = 0
        fakeBackend.lastSortField = ""
        fakeBackend.lastSortAscending = true
    }

    function init() {
        resetBackend()
        filterRequests = 0
        bar = barComponent.createObject(parent)
        verify(bar !== null, "SearchStatusBar 创建失败")
        bar.filterChanged.connect(function () {
            filterRequests += 1
        })
        menu = findChild(bar, "searchFilterMenu")
        verify(menu !== null, "缺少筛选菜单")
    }

    function cleanup() {
        if (menu)
            menu.close()
        menu = null
        if (bar)
            bar.destroy()
        bar = null
    }

    function filterButton() {
        return findChild(bar, "searchFilterButton")
    }

    function summaryLabel() {
        return findChild(bar, "searchFilterSummary")
    }

    function openMenu() {
        if (menu.visible)
            menu.close()
        wait(30)
        mouseClick(filterButton())
        wait(60)
        verify(menu.visible, "点筛选按钮应弹出菜单")
    }

    // 菜单项由 Repeater 生成，不属于 menu 的直接对象子级，用 itemAt 逐个找。
    // 现在委托本身就是 MenuItem（放在 Row/Column 里，不归 Menu 的菜单项管理）。
    function item(name) {
        var direct = findChild(menu, name)
        if (direct)
            return direct
        var repeaters = ["filterTypeRepeater", "filterModifiedRepeater", "filterSizeRepeater",
                         "filterSortFieldRepeater"]
        for (var r = 0; r < repeaters.length; ++r) {
            var repeater = findChild(menu, repeaters[r])
            if (!repeater)
                continue
            for (var i = 0; i < repeater.count; ++i) {
                var entry = repeater.itemAt(i)
                if (!entry)
                    continue
                if (entry.objectName === name)
                    return entry
                for (var c = 0; c < entry.children.length; ++c) {
                    if (entry.children[c].objectName === name)
                        return entry.children[c]
                }
            }
        }
        fail("菜单里缺少 " + name)
    }

    function test_button_and_summary_start_inactive() {
        compare(filterButton().text, "筛选", "没有筛选时按钮不带数字")
        verify(!summaryLabel().visible, "没有筛选时不显示摘要")
    }

    function test_button_shows_count_and_summary() {
        fakeBackend.searchFilterActiveCount = 2
        fakeBackend.searchFilterSummary = "视频 · 最近 7 天"
        wait(0)
        verify(filterButton().text.indexOf("2") >= 0,
               "激活条件数要写在按钮上：" + filterButton().text)
        verify(summaryLabel().visible, "有筛选时显示摘要")
        verify(summaryLabel().text.indexOf("视频 · 最近 7 天") >= 0,
               "摘要要写全：" + summaryLabel().text)
    }

    function test_menu_lists_three_groups_and_checks_current_values() {
        fakeBackend.searchFilterTypes = ["video"]
        fakeBackend.searchFilterModifieds = ["7d"]
        fakeBackend.searchFilterActiveCount = 2
        wait(0)
        openMenu()
        verify(item("filterType-video").checked, "当前类型要打勾")
        verify(!item("filterType-all").checked, "其它类型不打勾")
        verify(item("filterModified-7d").checked, "当前时间档要打勾")
        verify(!item("filterModified-any").checked)
        verify(item("filterSize-any").checked, "大小默认不限")
    }

    function test_picking_a_type_keeps_the_other_groups() {
        fakeBackend.searchFilterModifieds = ["30d"]
        fakeBackend.searchFilterSizes = ["10m-100m"]
        wait(0)
        openMenu()
        mouseClick(item("filterType-document"))
        wait(150)
        compare(fakeBackend.setCalls, 1)
        compare(fakeBackend.lastTypes.join(","), "document")
        compare(fakeBackend.lastModifieds.join(","), "30d", "改类型不能把时间筛选清掉")
        compare(fakeBackend.lastSizes.join(","), "10m-100m", "改类型不能把大小筛选清掉")
        compare(filterRequests, 1, "筛选变化要通知文件页重跑当前关键词")
        verify(menu.visible, "多选菜单勾完一项后保持打开")
    }

    function test_picking_size_reports_its_wire_name() {
        openMenu()
        mouseClick(item("filterSize-gt1g"))
        wait(150)
        compare(fakeBackend.lastSizes.join(","), "gt1g")
        compare(fakeBackend.lastTypes.length, 0)
        compare(fakeBackend.lastModifieds.length, 0)
    }

    function test_ticking_two_types_keeps_both_and_unticking_removes_one() {
        openMenu()
        mouseClick(item("filterType-video"))
        wait(150)
        mouseClick(item("filterType-audio"))
        wait(150)
        compare(fakeBackend.setCalls, 2)
        compare(fakeBackend.lastTypes.join(","), "video,audio", "同组多选取并集")
        verify(item("filterType-video").checked)
        verify(item("filterType-audio").checked)
        verify(!item("filterType-all").checked, "有勾选项时「全部」不该打勾")

        mouseClick(item("filterType-video"))
        wait(150)
        compare(fakeBackend.lastTypes.join(","), "audio", "再点一次取消该项")
    }

    function test_ticking_all_clears_the_group() {
        fakeBackend.searchFilterTypes = ["video", "audio"]
        fakeBackend.searchFilterActiveCount = 1
        wait(0)
        openMenu()
        verify(item("filterType-all") !== null, "缺少「全部」项")
        mouseClick(item("filterType-all"))
        wait(150)
        compare(fakeBackend.lastTypes.length, 0, "勾「全部」= 该组不限制")
        compare(fakeBackend.lastModifieds.length, 0)
        compare(fakeBackend.lastSizes.length, 0)
    }

    function test_sort_column_shows_direction_arrow_and_toggles_on_click() {
        openMenu()
        verify(findChild(bar, "filterSortMenu") === null, "不该再有排序二级菜单")
        verify(findChild(menu, "filterSortDirectionRepeater") === null,
               "升序 / 降序不再单独占行")
        verify(!item("filterSort-name").checkable, "排序行不显示勾选，只靠箭头表示当前列")
        verify(item("filterSort-name").text.indexOf("↑") >= 0,
               "当前列名后要显示方向箭头：" + item("filterSort-name").text)
        verify(item("filterSort-size").text.indexOf("↑") < 0
               && item("filterSort-size").text.indexOf("↓") < 0,
               "非当前列不带箭头")

        // 点别的列：换成那一列，用该列默认方向（大小 / 时间默认从大到小）
        mouseClick(item("filterSort-size"))
        wait(150)
        compare(fakeBackend.sortCalls, 1)
        compare(fakeBackend.lastSortField, "size")
        compare(fakeBackend.lastSortAscending, false, "大小默认从大到小")
        verify(item("filterSort-name").text.indexOf("↑") < 0, "箭头跟着当前列走")
        verify(item("filterSort-size").text.indexOf("↓") >= 0, "降序显示 ↓")
        verify(menu.visible, "改排序不收起菜单")

        // 再点当前列：只切换方向
        mouseClick(item("filterSort-size"))
        wait(150)
        compare(fakeBackend.sortCalls, 2)
        compare(fakeBackend.lastSortField, "size", "切换方向不能把排序依据丢掉")
        compare(fakeBackend.lastSortAscending, true)
        verify(item("filterSort-size").text.indexOf("↑") >= 0, "升序显示 ↑")

        // 名称列默认升序：点一下也直接切成降序
        mouseClick(item("filterSort-name"))
        wait(150)
        compare(fakeBackend.lastSortField, "name")
        compare(fakeBackend.lastSortAscending, true, "换列用该列默认方向（名称升序）")
        mouseClick(item("filterSort-name"))
        wait(150)
        compare(fakeBackend.lastSortAscending, false, "再点当前列切成降序")
        verify(item("filterSort-name").text.indexOf("↓") >= 0)
    }

    function test_check_marks_follow_external_changes() {
        openMenu()
        mouseClick(item("filterType-video"))
        wait(150)
        verify(item("filterType-video").checked, "点了之后要打勾")
        // 另一个入口（文件页面包屑上的筛选按钮）清掉了条件
        fakeBackend.searchFilterTypes = []
        fakeBackend.searchFilterActiveCount = 0
        wait(50)
        verify(!item("filterType-video").checked, "条件被别处清掉后勾选要同步")
        verify(item("filterType-all").checked, "空条件时「全部」要重新打勾")
    }

    function test_menu_items_highlight_on_hover() {
        openMenu()
        var target = item("filterType-video")
        mouseMove(target, target.width / 2, target.height / 2)
        wait(80)
        verify(target.highlighted, "鼠标悬停在筛选项上要有高亮反馈")
    }

    function test_menu_stays_inside_the_window() {
        openMenu()
        verify(menu.x >= 0, "菜单不能从左边出去：" + menu.x)
        verify(menu.x + menu.width <= width + 1,
               "菜单右缘要留在窗口内：" + (menu.x + menu.width) + " > " + width)
    }

    function test_clear_item_only_when_active() {        openMenu()
        verify(!item("searchFilterClearItem").visible, "没有筛选时不显示清除项")
        menu.close()
        wait(30)

        fakeBackend.searchFilterActiveCount = 1
        wait(0)
        openMenu()
        verify(item("searchFilterClearItem").visible, "有筛选时显示清除项")
        mouseClick(item("searchFilterClearItem"))
        wait(60)
        compare(fakeBackend.clearCalls, 1)
        compare(filterRequests, 1, "清除筛选也要重跑当前关键词")
    }

}
