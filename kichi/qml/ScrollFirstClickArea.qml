import QtQuick

// 可滚视图在惯性期间会把第一次左键按下当成“停止滚动”吞掉。
// 这一层只在 view.moving 时启用：按下先停住惯性并记录命中项，松开后
// 把首击补发给页面。正常情况下第二次点击回到原委托；惯性还没结束时，
// 这一层复用首击委托补发 activate，避免双击被吞掉。
//
// 滚轮、拖动和 KScrollBar 都不在这里处理；不要改成 WheelHandler 或手改 contentY。
Item {
    id: root

    property var view: null
    property int scrollBarSlot: 0
    /// 可选：(x, y) -> model index。网格用它排除卡片缝隙；默认走 view.indexAt。
    property var hitTest: null
    property int pressedIndex: -1
    property point pressedPoint: Qt.point(0, 0)
    /// 首击已经解析出的委托。惯性没及时停下时，第二击仍由这一层接住，
    /// 双击兜底必须复用同一个委托，不能假设 view 自己提供 itemAtIndex()。
    property var lastClickItem: null
    property int lastClickIndex: -1

    signal clickedAt(int index, int modifiers, point position)

    x: root.view ? root.view.x : 0
    y: root.view ? root.view.y : 0
    width: root.view ? Math.max(0, root.view.width - root.scrollBarSlot) : 0
    height: root.view ? root.view.height : 0
    z: 1

    function indexAt(x, y) {
        if (!root.view)
            return -1
        if (root.hitTest)
            return root.hitTest(x, y)
        return root.view.indexAt(x, y + root.view.contentY)
    }

    // itemAtIndex() 的静态类型只有 QQuickItem；具体委托的单击函数统一在这里
    // 以动态对象调用，避免 qmllint 把正常的委托接口当成缺失属性。
    function invokeFirstClick(item, index, modifiers, x, y) {
        if (!item) {
            root.lastClickItem = null
            root.lastClickIndex = -1
            return
        }
        root.lastClickItem = item
        root.lastClickIndex = index
        if (typeof item.firstClick === "function") {
            item.firstClick(modifiers, x, y)
            return
        }
        if (typeof item.selectRequested === "function")
            item.selectRequested(modifiers, index)
    }

    // 正常情况下第一次点击会 cancelFlick，第二次点击回到委托自己的
    // onDoubleClicked。但惯性偶尔不会在同一帧停下，第二次点击仍会落在这层；
    // 这时复用首击解析出的委托补发激活，否则表现为“左键双击没反应，右键菜单正常”。
    function invokeDoubleClick(item, x, y) {
        if (!item || typeof item.activate !== "function")
            return
        const local = root.mapToItem(item, x, y)
        if (local.x < 0 || local.y < 0
                || local.x >= item.width || local.y >= item.height)
            return
        item.activate(local)
    }

    MouseArea {
        anchors.fill: parent
        acceptedButtons: Qt.LeftButton
        enabled: root.view !== null && (root.view.moving || pressed)

        onPressed: function (mouse) {
            root.pressedIndex = root.indexAt(mouse.x, mouse.y)
            root.pressedPoint = Qt.point(mouse.x, mouse.y)
            if (root.view)
                root.view.cancelFlick()
        }

        onCanceled: {
            root.pressedIndex = -1
            root.lastClickItem = null
            root.lastClickIndex = -1
        }

        onClicked: function (mouse) {
            root.lastClickItem = null
            root.lastClickIndex = -1
            if (root.pressedIndex >= 0)
                root.clickedAt(root.pressedIndex, mouse.modifiers,
                               root.pressedPoint)
            root.pressedIndex = -1
        }

        onDoubleClicked: function (mouse) {
            if (!root.view || mouse.button !== Qt.LeftButton)
                return
            const index = root.indexAt(mouse.x, mouse.y)
            if (index < 0 || index !== root.lastClickIndex)
                return
            root.invokeDoubleClick(root.lastClickItem, mouse.x, mouse.y)
            root.lastClickItem = null
            root.lastClickIndex = -1
        }
    }
}
