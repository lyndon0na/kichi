import QtQuick
import QtTest
import ".."

// 文件页写操作弹窗（重命名 / 新建文件夹 / 批量回收站）。
//
// 为什么要有：QQC2 的 Dialog 配 `standardButtons` 时，点 Ok（AcceptRole）会
// 当场 close()；而这些都是异步请求，失败原因回来时对话框已经不可见，
// 用户只看到「点了确定就消失了」。这里锁住规则：
// 点确定不关、失败原因留在原地、成功才收起、请求进行中不能再点。
//
// 运行：
//   QT_QPA_PLATFORM=offscreen /usr/lib64/qt6/bin/qmltestrunner \
//       -input kichi/qml/tests/tst_files_menu.qml
TestCase {
    name: "FileActionDialogs"
    when: windowShown
    width: 900
    height: 600

    // 替代 Kichi 桥接对象：只提供弹窗用到的属性 / 方法 / 信号。
    QtObject {
        id: backend
        property bool fileActionRunning: false
        property int renameCount: 0
        property string renamedName: ""
        property int folderCount: 0
        property string createdName: ""
        property var lastTrashIds: []

        signal fileActionDone(string action, bool ok, string message)

        function renameEntry(id, name) {
            renameCount += 1
            renamedName = name
            fileActionRunning = true
        }
        function createFolder(name) {
            folderCount += 1
            createdName = name
            fileActionRunning = true
        }
        function trashEntries(ids) {
            lastTrashIds = ids.slice()
            fileActionRunning = true
        }
        /// 模拟 Rust 收尾：先清「进行中」，再发结果
        function finish(action, ok, message) {
            fileActionRunning = false
            fileActionDone(action, ok, message)
        }
    }

    Component {
        id: menuComponent
        FileActionDialogs {}
    }

    property var menu: null

    function init() {
        backend.fileActionRunning = false
        backend.renameCount = 0
        backend.renamedName = ""
        backend.folderCount = 0
        backend.createdName = ""
        backend.lastTrashIds = []
        menu = menuComponent.createObject(parent, {
            width: 600, height: 400, visible: true, backend: backend
        })
        verify(menu !== null, "FileActionDialogs 创建失败")
    }

    function cleanup() {
        if (menu)
            menu.destroy()
        menu = null
    }

    /// 打开重命名弹窗
    function startRename() {
        menu.openRename({ id: "id-1", name: "旧名字.mkv", kind: "file" })
        wait(60)
        var dialog = findChild(menu, "renameDialog")
        verify(dialog !== null && dialog.opened, "重命名对话框打开了")
        return dialog
    }

    function test_footer_button_order_matches_system() {
        // 两套页脚（DialogFooter / StandardDialogFooter）的「最右一个」都必须是取消，
        // 否则同一个位置在两类弹窗里做不同的事，容易误点。
        var dialog = startRename()
        var footer = dialog.footer
        var ok = findChild(footer, "dialogFooterOk")
        var cancel = findChild(footer, "dialogFooterCancel")
        verify(ok !== null && cancel !== null, "页脚有两个按钮")
        var okX = ok.mapToItem(footer, 0, 0).x
        var cancelX = cancel.mapToItem(footer, 0, 0).x
        verify(okX < cancelX,
               "确定应在左、取消在右（实际 ok.x=" + okX + " cancel.x=" + cancelX + "）")
    }

    function test_failed_rename_stays_open_with_the_reason() {
        var dialog = startRename()
        findChild(dialog, "renameNameField").text = "新名字.mkv"
        var ok = findChild(dialog, "dialogFooterOk")
        verify(ok !== null && ok.visible, "页脚有「确定」")
        mouseClick(ok)
        wait(40)

        compare(backend.renameCount, 1, "点确定只发一次改名请求")
        compare(backend.renamedName, "新名字.mkv")
        verify(dialog.opened, "请求还没回来：对话框不关")

        backend.finish("rename", false, "同名文件已存在")
        wait(40)
        verify(dialog.opened, "失败后对话框留在原地")
        var error = findChild(dialog, "renameError")
        verify(error !== null && error.visible, "失败原因看得到")
        compare(error.text, "同名文件已存在")
    }

    function test_successful_rename_closes_the_dialog() {
        var dialog = startRename()
        findChild(dialog, "renameNameField").text = "新名字.mkv"
        mouseClick(findChild(dialog, "dialogFooterOk"))
        wait(40)
        verify(dialog.opened, "成功之前仍然开着")

        backend.finish("rename", true, "已改名为「新名字.mkv」")
        wait(40)
        verify(!dialog.opened, "成功后收起")
    }

    function test_failed_new_folder_stays_open_with_the_reason() {
        menu.openNewFolder()
        wait(60)
        var dialog = findChild(menu, "folderDialog")
        verify(dialog !== null && dialog.opened, "新建文件夹对话框打开了")
        findChild(dialog, "folderNameField").text = "新文件夹"
        mouseClick(findChild(dialog, "dialogFooterOk"))
        wait(40)
        compare(backend.folderCount, 1, "点确定只发一次请求")
        compare(backend.createdName, "新文件夹")

        backend.finish("folder", false, "同名文件夹已存在")
        wait(40)
        verify(dialog.opened, "失败后对话框留在原地")
        var error = findChild(dialog, "folderError")
        verify(error !== null && error.visible, "失败原因看得到")
        compare(error.text, "同名文件夹已存在")
    }

    function test_ok_is_disabled_while_an_action_is_running() {
        backend.fileActionRunning = true
        var dialog = startRename()
        var ok = findChild(dialog, "dialogFooterOk")
        compare(ok.enabled, false, "请求进行中：确定置灰，防连点")
    }

    function test_unchanged_name_does_not_send_a_request() {
        var dialog = startRename()
        findChild(dialog, "renameNameField").text = "旧名字.mkv"
        mouseClick(findChild(dialog, "dialogFooterOk"))
        wait(40)
        compare(backend.renameCount, 0, "名字没变不发请求")
        verify(dialog.opened, "对话框也不关，用户可以接着改")
    }

    function test_dialog_is_centered_in_the_window() {
        // 宿主 Item 只有 600×400：弹窗必须按**窗口**居中，不能跟着宿主大小走。
        // 回归：拆组件后宿主成了 0×0，`anchors.centerIn: parent` 让弹窗
        // 跑到了窗口左上角（2026-10-28 实机）。
        menu.openNewFolder()
        wait(60)
        var dialog = findChild(menu, "folderDialog")
        verify(dialog !== null && dialog.opened, "新建文件夹对话框打开了")
        var dialogCenterX = dialog.x + dialog.width / 2
        verify(Math.abs(dialogCenterX - width / 2) <= 4,
               "水平居中于窗口（实际 " + dialogCenterX + "，期望 " + (width / 2) + "）")
        var dialogCenterY = dialog.y + dialog.height / 2
        verify(Math.abs(dialogCenterY - height / 2) <= 4,
               "垂直居中于窗口（实际 " + dialogCenterY + "，期望 " + (height / 2) + "）")
    }

    function test_batch_trash_shows_count_and_keeps_reason() {
        menu.openTrash({ id: "id-1", name: "a.mkv", kind: "file" },
                       ["id-1", "id-2"])
        wait(60)
        var dialog = findChild(menu, "trashDialog")
        verify(dialog !== null && dialog.opened, "回收站确认框打开了")
        verify(findChild(dialog, "trashMessage").text.indexOf("2 项") >= 0,
               "多选要显示条数")

        mouseClick(findChild(dialog, "dialogFooterOk"))
        wait(40)
        compare(backend.lastTrashIds.length, 2, "批量 id 一次交给后端")
        verify(dialog.opened, "请求还没回来：对话框不关")

        backend.finish("trash", false, "部分条目失败")
        wait(40)
        verify(dialog.opened, "失败后留在原地")
        compare(findChild(dialog, "trashError").text, "部分条目失败")

        backend.finish("trash", true, "已移入回收站")
        wait(40)
        verify(!dialog.opened, "成功后收起")
    }
}
