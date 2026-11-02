import QtQuick
import QtTest
import ".."

// 侧栏显示顺序与页面索引解耦，避免重排后点进错误页面。
TestCase {
    name: "SideBar"
    when: windowShown
    width: 320
    height: 700

    Component {
        id: sidebarComponent

        SideBar {
            backend: testBackend
            width: 208
            height: 680
        }
    }

    QtObject {
        id: testBackend

        property var tasks: []
        property string account: "tester"
        property string displayName: "Tester"
        property string avatarPath: ""
        property string quotaUsed: "0 B"
        property string quotaLimit: "1 GB"
        property bool quotaLoading: false
        property double quotaRatio: 0
        property bool quotaVip: false
    }

    property var sidebar: null

    SignalSpy {
        id: pageSpy
        signalName: "pageRequested"
    }

    SignalSpy {
        id: settingsSpy
        signalName: "settingsRequested"
    }

    Connections {
        target: sidebar

        function onPageRequested(pageIndex) {
            sidebar.currentIndex = pageIndex
        }
    }

    function init() {
        sidebar = sidebarComponent.createObject(parent, {
            backend: testBackend,
            currentIndex: 0,
            visible: true
        })
        verify(sidebar !== null, "SideBar 创建失败")
        pageSpy.target = sidebar
        settingsSpy.target = sidebar
    }

    function cleanup() {
        pageSpy.target = null
        settingsSpy.target = null
        if (sidebar)
            sidebar.destroy()
        sidebar = null
    }

    function navItem(page) {
        return findChild(sidebar, "sidebarNav_" + page)
    }

    function test_order_and_page_mapping() {
        const expectedPages = [0, 6, 7, 8, 3, 1, 4]
        let previousY = -1

        for (let i = 0; i < expectedPages.length; ++i) {
            const page = expectedPages[i]
            const item = navItem(page)
            verify(item !== null, "缺少页面 " + page + " 的侧栏入口")

            const y = item.mapToItem(sidebar, 0, 0).y
            verify(y > previousY, "页面 " + page + " 的侧栏顺序不正确")
            previousY = y

            pageSpy.clear()
            item.clicked()
            compare(pageSpy.count, 1)
            compare(pageSpy.signalArguments[0][0], page)
            wait(0)
        }

        compare(navItem(2), null, "传输任务不再占主导航")
        compare(navItem(5), null, "设置不再占主导航")
    }

    function test_repeated_click_keeps_current_page_selected() {
        const page = 6
        const item = navItem(page)
        verify(item !== null)
        compare(item.checkable, false, "导航按钮不能自行切换勾选")

        sidebar.currentIndex = 0
        pageSpy.clear()

        mouseClick(item, 4, item.height / 2)
        wait(0)
        compare(pageSpy.count, 1)
        compare(sidebar.currentIndex, page)
        compare(item.navSelected, true, "第一次点击后当前页应保持选中")

        mouseClick(item, 4, item.height / 2)
        wait(0)
        compare(pageSpy.count, 2)
        compare(sidebar.currentIndex, page)
        compare(item.navSelected, true, "再次点击当前入口不能取消选中")

        sidebar.currentIndex = 0
    }

    function test_account_settings_entry_requests_settings_page() {
        const settingsButton = findChild(sidebar, "accountSettingsButton")
        verify(settingsButton !== null, "账号区缺少设置入口")

        settingsSpy.clear()
        settingsButton.clicked()
        compare(settingsSpy.count, 1)
    }

    function test_offline_task_badge_counts_only_unfinished_tasks() {
        const badge = findChild(sidebar, "sidebarTaskBadge")
        verify(badge !== null, "侧栏缺少离线任务数量徽标")

        testBackend.tasks = [
            { settled: false },
            { settled: true },
            { settled: false }
        ]
        compare(sidebar.unfinishedTaskCount, 2)
        compare(badge.count, 2)
        compare(badge.countVisible, true)
        compare(badge.text, "2")

        testBackend.tasks = [
            { settled: true },
            { settled: true }
        ]
        compare(badge.count, 0)
        compare(badge.countVisible, false)

        const many = []
        for (let i = 0; i < 105; ++i)
            many.push({ settled: false })
        testBackend.tasks = many
        compare(badge.count, 105)
        compare(badge.countVisible, true)
        compare(badge.text, "99+")
    }
}
