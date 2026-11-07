import QtQuick
import QtTest
import "../files"

// 文件页右键「排序」子菜单。
//
// 网格视图没有表头可点，排序只能从这里进，所以锁住四件事：
// ① 打勾反映后端当前排序（每次打开重读，不靠上一次点击留下的状态）；
// ② 换排序列用该列的常用方向（名称升序，大小 / 修改时间降序）——和表头同一条规则；
// ③ 只改方向时排序列不变；
// ④ 点完当场把打勾同步干净，别剩两个勾选。
//
// 运行：
//   QT_QPA_PLATFORM=offscreen /usr/lib64/qt6/bin/qmltestrunner \
//       -input kichi/qml/tests/tst_file_sort_menu.qml
TestCase {
    name: "FileSortMenu"
    when: windowShown
    width: 900
    height: 600

    // 替代 Kichi 桥接对象：只提供排序用到的属性 / 方法。
    QtObject {
        id: backend
        property bool filesLoading: false
        property bool fileActionRunning: false
        property var fileActionIds: []
        property bool trashClearing: false
        property bool shareActionRunning: false
        property var playMenu: []
        property string sortField: "name"
        property bool sortAscending: true
        property int sortCount: 0
        property string lastField: ""
        property bool lastAscending: true

        signal fileActionDone(string action, bool ok, string message, var ids)

        function isDownloadPending(id) {
            return false
        }
        function hasPartialDownload(id) {
            return false
        }
        /// 与 Rust 同一条语义：状态没变直接返回，变了才落到属性上
        function setSorting(field, ascending) {
            if (sortField === field && sortAscending === ascending)
                return
            sortCount += 1
            lastField = field
            lastAscending = ascending
            sortField = field
            sortAscending = ascending
        }
    }

    Component {
        id: menuComponent
        FilesPageMenu {}
    }

    property var menu: null
    property var sortMenu: null

    function init() {
        backend.sortField = "name"
        backend.sortAscending = true
        backend.sortCount = 0
        backend.lastField = ""
        backend.lastAscending = true
        menu = menuComponent.createObject(parent, {
            width: 600, height: 400, visible: true, backend: backend
        })
        verify(menu !== null, "FilesPageMenu 创建失败")
    }

    function cleanup() {
        if (sortMenu)
            sortMenu.close()
        sortMenu = null
        if (menu)
            menu.destroy()
        menu = null
    }

    /// 弹空白菜单，再弹里面的排序子菜单（打勾在子菜单自己弹出时才同步）
    ///
    /// 先把上一次还开着的菜单收掉：org.kde.desktop 风格点完一项会把整个菜单关掉，
    /// 不收干净就重开，QtTest 会在「窗口还没暴露」的旧窗口上点空。
    function openSort() {
        if (sortMenu)
            sortMenu.close()
        var blank = findChild(menu, "fileBlankMenu")
        verify(blank !== null, "有空白处右键菜单")
        blank.close()
        wait(60)
        menu.openBlankAt(Qt.point(20, 20))
        wait(60)
        sortMenu = findChild(menu, "blankSortMenu")
        verify(sortMenu !== null, "空白菜单里有排序子菜单")
        sortMenu.open()
        wait(60)
    }

    function mark(name) {
        var item = findChild(sortMenu, name)
        verify(item !== null, "子菜单里有 " + name)
        return item.checked
    }

    function test_a_shows_the_current_sorting_on_open() {
        openSort()
        verify(mark("sortByNameItem"), "名称是当前排序列")
        verify(!mark("sortBySizeItem"), "大小未选中")
        verify(!mark("sortByModifiedItem"), "修改时间未选中")
        verify(mark("sortAscendingItem"), "当前是升序")
        verify(!mark("sortDescendingItem"), "降序未选中")
    }

    function test_b_reflects_changes_made_elsewhere() {
        // 列表视图的点表头是另一个入口：菜单重开时必须跟随后端状态
        backend.sortField = "modified"
        backend.sortAscending = false
        openSort()
        verify(mark("sortByModifiedItem"), "修改时间是当前排序列")
        verify(!mark("sortByNameItem"), "名称不再打勾")
        verify(mark("sortDescendingItem"), "当前是降序")
    }

    function test_c_choosing_a_field_uses_its_default_direction() {
        openSort()
        mouseClick(findChild(sortMenu, "sortBySizeItem"))
        wait(40)
        compare(backend.lastField, "size", "换到大小列")
        compare(backend.lastAscending, false, "大小用降序（大的在前）")
        verify(mark("sortBySizeItem"), "大小当场打勾")
        verify(!mark("sortByNameItem"), "名称的勾让出来，不能两个都勾")
        verify(mark("sortDescendingItem"), "方向跟着换成降序")
    }

    function test_d_direction_keeps_the_current_field() {
        openSort()
        mouseClick(findChild(sortMenu, "sortDescendingItem"))
        wait(40)
        compare(backend.lastField, "name", "只改方向，排序列不动")
        compare(backend.lastAscending, false)
        compare(backend.sortCount, 1, "一次点击只发一次排序")
        verify(mark("sortDescendingItem"), "降序打勾")
        verify(!mark("sortAscendingItem"), "升序让出勾")
    }

    function test_e_clicking_the_current_field_is_a_no_op() {
        openSort()
        mouseClick(findChild(sortMenu, "sortByNameItem"))
        wait(40)
        compare(backend.sortCount, 0, "已是当前排序列时不重发")
        verify(mark("sortByNameItem"), "名称的勾没被点击本身弄掉")
    }

    // org.kde.desktop 风格点完一项会把整条菜单关掉，同一个测试里重开第二次会点到
    // 还没暴露的旧窗口，所以「点当前方向」另起一个用例（每个用例都重建菜单）。
    function test_f_clicking_the_current_direction_is_a_no_op() {
        openSort()
        mouseClick(findChild(sortMenu, "sortAscendingItem"))
        wait(40)
        compare(backend.sortCount, 0, "已是当前方向时不重发")
        verify(mark("sortAscendingItem"), "升序的勾没被点击本身弄掉")
    }
}
