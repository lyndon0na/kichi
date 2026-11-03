import QtQuick
import QtTest
import ".."

// 底栏任务入口：空闲文案稳定、本机传输与离线任务都计入数量并发出切页请求。
TestCase {
    name: "TransferStatusButton"
    when: windowShown
    width: 280
    height: 80

    Component {
        id: buttonComponent

        TransferStatusButton {
            activeCount: 0
        }
    }

    SignalSpy {
        id: requestSpy
        signalName: "requested"
    }

    property var button: null

    function init() {
        button = buttonComponent.createObject(parent, { "visible": true })
        requestSpy.target = button
    }

    function cleanup() {
        requestSpy.target = null
        if (button)
            button.destroy()
        button = null
    }

    function countBadge() {
        return findChild(button, "transferStatusCount")
    }

    function test_idle_state_has_text_and_no_badge() {
        button.activeCount = 0
        button.offlineCount = 0
        wait(0)

        compare(button.text, "任务")
        compare(countBadge().visible, false)
    }

    function test_active_count_is_visible_and_capped() {
        button.activeCount = 3
        button.offlineCount = 0
        wait(0)
        compare(button.activeCount, 3)
        compare(button.totalCount, 3)
        compare(button.visible, true)
        compare(countBadge().text, "3")
        compare(countBadge().visible, true)

        button.activeCount = 120
        wait(0)
        compare(countBadge().text, "99+")
    }

    function test_offline_count_is_added_to_badge() {
        button.activeCount = 2
        button.offlineCount = 3
        wait(0)

        compare(button.totalCount, 5)
        compare(countBadge().text, "5")
        compare(countBadge().visible, true)
    }

    function test_click_requests_transfer_page() {
        requestSpy.clear()
        button.clicked()
        compare(requestSpy.count, 1)
    }
}
