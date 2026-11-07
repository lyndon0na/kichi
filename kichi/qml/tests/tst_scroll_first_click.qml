import QtQuick
import QtTest
import "../components"

// 首击转发层自己的双击兜底：
// 正常路径是第一次点击停住惯性、第二次回到委托；但如果视图仍报告 moving，
// 两次点击都会落在这层上，必须由它补发 activate，不能把双击吞掉。
TestCase {
    name: "ScrollFirstClick"
    when: windowShown
    width: 400
    height: 400

    property int activationCalls: 0
    property int rowActivations: 0
    property bool viewMoving: true
    property var eventOrder: []

    Component {
        id: hostComponent

        Item {
            id: host
            width: 300
            height: 300

            Item {
                id: fakeView
                objectName: "fakeView"
                width: 300
                height: 300
                property bool moving: viewMoving
                property real contentY: 0

                function indexAt(x, y) {
                    return y >= 0 && y < fakeRow.height ? 0 : -1
                }

                function cancelFlick() {}

                Item {
                    id: fakeRow
                    width: 300
                    height: 48

                    function activate(point) {
                        activationCalls += 1
                        eventOrder.push("activate")
                    }

                    function firstClick(modifiers, x, y) {
                        eventOrder.push("select")
                    }

                    MouseArea {
                        anchors.fill: parent
                        acceptedButtons: Qt.LeftButton
                        onDoubleClicked: function (mouse) {
                            rowActivations += 1
                            fakeRow.activate(Qt.point(mouse.x, mouse.y))
                        }
                    }
                }
            }

            ScrollFirstClickArea {
                id: firstClickArea
                view: fakeView
                scrollBarSlot: 0

                onClickedAt: function (index, modifiers, position) {
                    const item = index === 0 ? fakeRow : null
                    if (item)
                        firstClickArea.invokeFirstClick(
                            item, index, modifiers, position.x, position.y)
                }
            }
        }
    }

    property var host: null

    function init() {
        activationCalls = 0
        rowActivations = 0
        eventOrder = []
        viewMoving = true
        host = hostComponent.createObject(parent, { visible: true })
        wait(30)
    }

    function cleanup() {
        if (host)
            host.destroy()
        wait(30)
        host = null
    }

    function test_double_click_is_forwarded_when_view_stays_moving() {
        const view = findChild(host, "fakeView")
        verify(view !== null)
        mouseDoubleClickSequence(view, 80, 20)
        wait(30)
        compare(activationCalls, 1,
                "两次点击都被首击层接住时仍必须激活一次")
        compare(rowActivations, 0, "委托不应同时收到第二次点击")
        compare(eventOrder[eventOrder.length - 1], "activate",
                "先完成选择再激活，避免目录切换后误选新行")
    }

    function test_double_click_uses_row_when_view_is_still() {
        viewMoving = false
        wait(30)
        const view = findChild(host, "fakeView")
        verify(view !== null)
        compare(view.moving, false)
        mouseDoubleClickSequence(view, 80, 20)
        wait(30)
        compare(activationCalls, 1, "静止时仍必须激活一次")
        compare(rowActivations, 1, "静止时仍走委托自己的双击")
    }
}
