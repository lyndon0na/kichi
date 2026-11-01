import QtQuick
import QtTest
import ".."

// 底栏传输入口：空闲文案稳定、进行中显示数量并发出切页请求。
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
        wait(0)

        compare(button.text, "传输任务")
        compare(countBadge().visible, false)
    }

    function test_active_count_is_visible_and_capped() {
        button.activeCount = 3
        wait(0)
        compare(button.activeCount, 3)
        compare(button.visible, true)
        compare(countBadge().text, "3")
        compare(countBadge().visible, true)

        button.activeCount = 120
        wait(0)
        compare(countBadge().text, "99+")
    }

    function test_click_requests_transfer_page() {
        requestSpy.clear()
        button.clicked()
        compare(requestSpy.count, 1)
    }
}
