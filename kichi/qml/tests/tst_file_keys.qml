import QtQuick
import QtTest
import ".."

// 文件页快捷键的焦点前提（用户实机报过「Ctrl+A / Delete / F2 / Enter / Backspace
// 按了都没反应」）：
//
// 1. 列表创建时如果是不可见的（登录页在 StackLayout 里占着当前页），
//    `focus: true` 拿不到 activeFocus，登录进来后 Qt 也不会自动补；
// 2. `ListView.view` 附加属性只在委托根上有效，写在下层 MouseArea 里是 null，
//    所以「点行 → 把焦点交给列表」必须在 FileRow 根上取 view。
//
// 这里用 `focus: false` 模拟「焦点在别处」，验证点行后焦点回到列表、按键能到达。
TestCase {
    name: "FileListKeys"
    when: windowShown
    width: 400
    height: 300

    property int keysSeen: 0
    property var list: null

    Component {
        id: listComponent

        ListView {
            id: list
            width: 380
            height: 280
            // 故意不自动拿焦点：焦点先前在登录页的输入框上
            focus: false
            model: [
                { id: "a", name: "a.mkv", kind: "file", playable: true,
                  sizeText: "1 MB", modifiedText: "今天", auditFlag: false },
                { id: "b", name: "b", kind: "folder", playable: false,
                  sizeText: "", modifiedText: "", auditFlag: false },
            ]
            Keys.onPressed: function (event) {
                keysSeen += 1
                event.accepted = true
            }
            delegate: FileRow {
                width: ListView.view.width
                height: 40
                entry: modelData
            }
        }
    }

    function init() {
        keysSeen = 0
        list = listComponent.createObject(parent, { visible: true })
        wait(50)
    }

    function cleanup() {
        if (list)
            list.destroy()
        list = null
    }

    function test_click_returns_focus_to_list() {
        compare(list.activeFocus, false, "初始没有焦点（模拟登录页占着焦点）")

        // 点第一行：FileRow 的整行 MouseArea 必须把焦点交还给列表
        mouseClick(list, 100, 20)
        wait(30)

        var row = list.itemAtIndex(0)
        verify(row !== null)
        compare(row.listView !== null, true, "委托根能拿到 ListView.view")
        compare(list.activeFocus, true, "点行后焦点回到列表")

        keyPress(Qt.Key_Delete)
        wait(30)
        compare(keysSeen >= 1, true, "焦点回来后按键能到达列表")

        keyPress(Qt.Key_A, Qt.ControlModifier)
        wait(30)
        compare(keysSeen >= 2, true, "Ctrl+A 也能到达列表")
    }
}
