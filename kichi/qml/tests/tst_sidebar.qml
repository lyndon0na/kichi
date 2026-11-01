import QtQuick
import QtTest
import ".."

// 侧栏显示顺序与页面索引解耦，避免重排后点进错误页面。
TestCase {
    name: "SideBar"
    when: windowShown
    width: 320
    height: 700

    SideBar {
        id: sidebar
        width: 208
        height: 680
    }

    SignalSpy {
        id: pageSpy
        target: sidebar
        signalName: "pageRequested"
    }

    SignalSpy {
        id: settingsSpy
        target: sidebar
        signalName: "settingsRequested"
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

    function test_account_settings_entry_requests_settings_page() {
        const settingsButton = findChild(sidebar, "accountSettingsButton")
        verify(settingsButton !== null, "账号区缺少设置入口")

        settingsSpy.clear()
        settingsButton.clicked()
        compare(settingsSpy.count, 1)
    }
}
