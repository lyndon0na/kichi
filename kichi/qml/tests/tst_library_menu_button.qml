import QtQuick
import QtTest
import ".."

// 资料库入口（方案 C）：文件页和库页都能打开；打勾跟随当前页；
// 按钮只在库页保持强调态，点击项只发页面请求，不改自身状态。
TestCase {
    name: "LibraryMenuButton"
    when: windowShown
    width: 400
    height: 300

    Component {
        id: buttonComponent

        LibraryMenuButton {}
    }

    PageIndex { id: pages }

    property var button: null
    property var menu: null

    SignalSpy {
        id: pageSpy
        signalName: "pageRequested"
    }

    function init() {
        button = buttonComponent.createObject(parent, {
            visible: true,
            currentPage: 0
        })
        verify(button !== null, "LibraryMenuButton 创建失败")
        menu = button.menu
        verify(menu !== null, "缺少资料库菜单")
        pageSpy.target = button
    }

    function cleanup() {
        pageSpy.target = null
        if (menu)
            menu.close()
        menu = null
        if (button)
            button.destroy()
        button = null
    }

    function openMenu() {
        button.openMenu()
        wait(50)
        verify(menu.opened, "资料库菜单没有打开")
    }

    function test_button_active_only_on_library_pages() {
        compare(button.pageActive, false)

        button.currentPage = pages.playbackHistory
        compare(button.pageActive, true, "库页按钮应保持强调色")

        button.currentPage = 0
        compare(button.pageActive, false)
    }

    function test_menu_marks_the_current_library() {
        openMenu()
        verify(!findChild(menu, "libraryMenuRecentSaved").checked)
        verify(!findChild(menu, "libraryMenuPlaybackHistory").checked)
        verify(!findChild(menu, "libraryMenuStarred").checked)
        menu.close()
        wait(20)

        button.currentPage = pages.playbackHistory
        openMenu()
        verify(findChild(menu, "libraryMenuPlaybackHistory").checked,
               "播放历史应打勾")
        verify(!findChild(menu, "libraryMenuRecentSaved").checked)
        verify(!findChild(menu, "libraryMenuStarred").checked)
    }

    function test_items_request_the_matching_page() {
        openMenu()
        const cases = [
            ["libraryMenuRecentSaved", pages.recentSaved],
            ["libraryMenuPlaybackHistory", pages.playbackHistory],
            ["libraryMenuStarred", pages.starred]
        ]
        for (const [name, page] of cases) {
            pageSpy.clear()
            findChild(menu, name).triggered()
            compare(pageSpy.count, 1, name + " 应发出一次页面请求")
            compare(pageSpy.signalArguments[0][0], page)
        }
    }
}
