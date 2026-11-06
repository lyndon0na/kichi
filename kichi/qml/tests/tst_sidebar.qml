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

        property string account: "tester"
        property string displayName: "Tester"
        property string avatarPath: ""
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
        // 传输任务紧跟我的文件；三个资料库收进文件页，设置收进账号行。
        const expectedPages = [0, 2, 1, 3]
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

        compare(navItem(4), null, "最近保存不占主导航")
        compare(navItem(5), null, "播放历史不占主导航")
        compare(navItem(6), null, "已加星标不占主导航")
    }

    function test_task_badge_only_shows_unfinished_count() {
        const badge = findChild(sidebar, "sidebarTaskBadge")
        verify(badge !== null, "传输任务入口缺少数量徽标")
        compare(badge.visible, false)

        sidebar.localTaskCount = 1
        sidebar.offlineTaskCount = 2
        wait(0)
        compare(badge.visible, true)
        compare(badge.text, "3")

        sidebar.localTaskCount = 0
        sidebar.offlineTaskCount = 120
        wait(0)
        compare(badge.text, "99+")

        sidebar.offlineTaskCount = 0
        wait(0)
        compare(badge.visible, false)
    }

    function test_repeated_click_keeps_current_page_selected() {
        const page = 1
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
        verify(settingsButton !== null, "账号行缺少设置入口")
        compare(settingsButton.checkable, false, "设置按钮不能自行切换勾选")

        settingsSpy.clear()
        settingsButton.clicked()
        compare(settingsSpy.count, 1)
    }

    function test_selection_surface_follows_current_page() {
        const filesSurface = findChild(navItem(0), "sidebarSelectionSurface")
        const targetSurface = findChild(navItem(1), "sidebarSelectionSurface")
        const selectionBar = findChild(sidebar, "sidebarSelectionBar")
        verify(filesSurface !== null, "当前页缺少选中底色")
        verify(targetSurface !== null, "目标页缺少选中底色")
        verify(selectionBar !== null, "侧栏缺少滑动选中竖条")

        sidebar.currentIndex = 0
        tryCompare(filesSurface, "opacity", 1, 1000)
        tryCompare(targetSurface, "opacity", 0, 1000)
        compare(selectionBar.visible, true)

        sidebar.currentIndex = 1
        tryCompare(targetSurface, "opacity", 1, 1000)
        tryCompare(filesSurface, "opacity", 0, 1000)

        const target = navItem(1)
        const expectedY = target.mapToItem(
            sidebar, 0, (target.height - selectionBar.height) / 2).y
        tryCompare(selectionBar, "y", expectedY, 1000)

        sidebar.currentIndex = 2
        const tasksItem = navItem(2)
        verify(tasksItem.navSelected, "传输任务页应选中侧栏入口")
        compare(selectionBar.visible, true)
        const tasksY = tasksItem.mapToItem(
            sidebar, 0, (tasksItem.height - selectionBar.height) / 2).y
        tryCompare(selectionBar, "y", tasksY, 1000)

        // 库页属于文件页：侧栏仍选中「我的文件」
        sidebar.currentIndex = 4
        tryCompare(filesSurface, "opacity", 1, 1000)
        tryCompare(targetSurface, "opacity", 0, 1000)
        verify(navItem(0).navSelected, "最近保存页应选中我的文件")
        const filesTarget = navItem(0)
        const filesY = filesTarget.mapToItem(
            sidebar, 0, (filesTarget.height - selectionBar.height) / 2).y
        tryCompare(selectionBar, "y", filesY, 1000)

        // 设置是独立窗口：不占页面索引，齿轮点亮只看 settingsActive
        const settingsButton = findChild(sidebar, "accountSettingsButton")
        sidebar.settingsActive = true
        wait(0)
        compare(settingsButton.checked, true, "设置对话框打开时应点亮账号行设置按钮")
        compare(selectionBar.visible, true, "主导航仍保持当前页的选中竖条")
        verify(navItem(0).navSelected, "打开设置不应改变主导航选中项")

        sidebar.settingsActive = false
        sidebar.currentIndex = 0
        wait(0)
        compare(settingsButton.checked, false)
    }
}
