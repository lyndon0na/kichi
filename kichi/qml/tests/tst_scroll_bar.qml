import QtQuick
import QtQuick.Controls
import QtTest
import "../components"

// KScrollBar：内容装不下时静止也在位；KDE 风格背景（整条轨道线）必须清掉，
// 只留 6px 滑块，否则右侧看起来像还留着一根车道边框。
TestCase {
    name: "ScrollBarStyle"
    when: windowShown
    width: 400
    height: 300

    Component {
        id: hostComponent

        ListView {
            id: host

            anchors.fill: parent
            model: 100
            delegate: Item {
                width: ListView.view.width
                height: 30
            }
            boundsBehavior: Flickable.StopAtBounds
            ScrollBar.vertical: KScrollBar {
                id: bar
                objectName: "barProbe"
            }
        }
    }

    property var host: null
    property var bar: null

    function initTestCase() {
        host = hostComponent.createObject(parent)
        verify(host !== null)
        wait(100)
        bar = findChild(host, "barProbe")
        verify(bar !== null)
    }

    function test_bar_stays_visible_at_rest() {
        wait(100)
        compare(bar.size < 1, true, "测试内容应超出可视区")
        compare(bar.opacity, 1, "静止时条也要在位")
        compare(bar.background, null, "KDE 原生轨道线必须清掉")
        compare(bar.implicitWidth, 10)
        compare(bar.contentItem.width, 6, "滑块本体 6px")
    }

    function test_delegate_fills_view_width() {
        const first = host.itemAtIndex(0)
        verify(first !== null)
        compare(first.width, host.width)
    }

    // 清掉 KDE 背景后，滑块拖动仍要能滚动（拖拽由 ScrollBar 自身处理）
    function test_drag_handle_scrolls() {
        host.cancelFlick()
        host.contentY = 0
        wait(50)
        const x = host.width - bar.width / 2
        const handleY = Math.max(1, bar.visualSize * host.height / 2)
        mousePress(host, x, handleY)
        mouseMove(host, x, handleY + 120, 60)
        mouseRelease(host, x, handleY + 120)
        wait(100)
        verify(host.contentY > 0,
               "拖动滑块必须滚动列表，contentY=" + host.contentY)
    }
}
