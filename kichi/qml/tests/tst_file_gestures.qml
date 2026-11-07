import QtQuick
import QtTest

// 文件列表的右键路由：
// - 行上的右键由行内的 MouseArea 处理（多选需要修饰键）；
// - 空白处由 ListView 级别的 TapHandler 接住，不能把行的右键也当成空白。
//
// 这两个处理器叠在一起时很容易变成「弹出两个菜单」或「行右键没反应」，
// 所以用最小复现锁住行为。
TestCase {
    name: "FileListGestures"
    when: windowShown
    width: 400
    height: 400

    property int blankTaps: 0
    property int rowTaps: 0

    Component {
        id: listComponent

        ListView {
            id: list

            width: 300
            height: 300
            model: 3

            delegate: Item {
                width: ListView.view.width
                height: 40

                MouseArea {
                    anchors.fill: parent
                    acceptedButtons: Qt.LeftButton | Qt.RightButton
                    onClicked: rowTaps += 1
                }
            }

            TapHandler {
                acceptedButtons: Qt.RightButton
                onTapped: function (point) {
                    // 挂在 Flickable 上的 handler，position 是内容坐标，
                    // 先换算回视口坐标。
                    const viewport = list.mapFromItem(
                        null, point.scenePosition.x, point.scenePosition.y)
                    if (list.indexAt(viewport.x,
                                     viewport.y + list.contentY) < 0)
                        blankTaps += 1
                }
            }
        }
    }

    property var list: null

    function init() {
        blankTaps = 0
        rowTaps = 0
        list = listComponent.createObject(parent, { visible: true })
        wait(50)
    }

    function cleanup() {
        if (list)
            list.destroy()
        list = null
    }

    function test_row_right_click_is_handled_by_the_row_only() {
        // 第 2 行（y = 40..80）
        mouseClick(list, 100, 60, Qt.RightButton)
        wait(30)
        compare(rowTaps, 1, "行自己处理右键")
        compare(blankTaps, 0, "列表不能把行的右键当成空白")
    }

    function test_blank_right_click_is_handled_by_the_list() {
        // 3 行共 120px，下面全是空白
        mouseClick(list, 100, 250, Qt.RightButton)
        wait(30)
        compare(blankTaps, 1, "空白右键由列表接住")
        compare(rowTaps, 0)
    }
}
