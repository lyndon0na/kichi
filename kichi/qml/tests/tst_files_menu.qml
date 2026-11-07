import QtQuick
import QtTest
import "../components"
import "../files"

// 文件页写操作弹窗（重命名 / 新建文件夹 / 删除）。
//
// 文件写入先乐观更新，Rust 接受请求后弹窗立即关闭；网络失败由底栏提示，
// 不能因为等待响应继续挡住界面。这里同时锁住同步校验失败仍留在弹窗。
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
        property bool filesLoading: false
        property bool fileActionRunning: false
        property var fileActionIds: []
        property bool trashClearing: false
        property int renameCount: 0
        property string renamedName: ""
        property int folderCount: 0
        property string createdName: ""
        property var lastTrashIds: []
        property var lastDeleteIds: []

        signal fileActionDone(string action, bool ok, string message, var ids)
        signal folderCreateStarted(string id)

        function renameEntry(id, name) {
            renameCount += 1
            renamedName = name
            fileActionIds = [id]
            return true
        }
        function createFolder(name) {
            folderCount += 1
            createdName = name
            fileActionIds = ["kichi-pending-folder-1"]
            folderCreateStarted("kichi-pending-folder-1")
            return true
        }
        function trashEntries(ids) {
            lastTrashIds = ids.slice()
            fileActionIds = ids.slice()
            return true
        }
        function deleteEntries(ids) {
            lastDeleteIds = ids.slice()
            fileActionIds = ids.slice()
            return true
        }
        /// 模拟 Rust 收尾：先清「进行中」，再发结果
        function finish(action, ok, message, ids) {
            fileActionIds = []
            fileActionDone(action, ok, message, ids || [])
        }
    }

    Component {
        id: menuComponent
        FileActionDialogs {}
    }

    property var menu: null

    function init() {
        backend.fileActionRunning = false
        backend.fileActionIds = []
        backend.renameCount = 0
        backend.renamedName = ""
        backend.folderCount = 0
        backend.createdName = ""
        backend.lastTrashIds = []
        backend.lastDeleteIds = []
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
        // 全应用确认弹窗共用 DialogFooter，「最右一个」必须是取消，
        // 否则同一个位置在不同弹窗里做不同的事，容易误点。
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

    function test_rename_closes_immediately_and_failure_does_not_reopen() {
        var dialog = startRename()
        findChild(dialog, "renameNameField").text = "新名字.mkv"
        var ok = findChild(dialog, "dialogFooterOk")
        verify(ok !== null && ok.visible, "页脚有「确定」")
        mouseClick(ok)
        wait(40)

        compare(backend.renameCount, 1, "点确定只发一次改名请求")
        compare(backend.renamedName, "新名字.mkv")
        verify(!dialog.opened, "本地条目已更新，请求发出后立即关窗")

        backend.finish("rename", false, "同名文件已存在")
        wait(40)
        verify(!dialog.opened, "网络失败由底栏提示，不能重新弹窗")
    }

    function test_successful_rename_closes_the_dialog() {
        var dialog = startRename()
        findChild(dialog, "renameNameField").text = "新名字.mkv"
        mouseClick(findChild(dialog, "dialogFooterOk"))
        wait(40)
        verify(!dialog.opened, "发出请求后立即收起")

        backend.finish("rename", true, "已改名为「新名字.mkv」")
        wait(40)
        verify(!dialog.opened, "成功后保持关闭")
    }

    function test_new_folder_closes_immediately_even_if_network_fails() {
        menu.openNewFolder()
        wait(60)
        var dialog = findChild(menu, "folderDialog")
        verify(dialog !== null && dialog.opened, "新建文件夹对话框打开了")
        findChild(dialog, "folderNameField").text = "新文件夹"
        mouseClick(findChild(dialog, "dialogFooterOk"))
        wait(40)
        compare(backend.folderCount, 1, "点确定只发一次请求")
        compare(backend.createdName, "新文件夹")
        verify(!dialog.opened, "临时文件夹已插入，立即关窗")

        backend.finish("folder", false, "同名文件夹已存在")
        wait(40)
        verify(!dialog.opened, "失败只回滚并走底栏提示")
    }

    function test_old_folder_result_does_not_close_a_new_dialog() {
        menu.openNewFolder()
        wait(60)
        var first = findChild(menu, "folderDialog")
        findChild(first, "folderNameField").text = "第一间"
        mouseClick(findChild(first, "dialogFooterOk"))
        wait(40)
        verify(!first.opened, "第一间发出请求后立即关窗")

        menu.openNewFolder()
        wait(60)
        var second = findChild(menu, "folderDialog")
        verify(second.opened, "新确认框已打开")
        compare(findChild(second, "folderError").text, "")

        backend.finish("folder", false, "旧请求失败", ["kichi-pending-folder-1"])
        wait(40)
        verify(second.opened, "旧请求结果不能关掉新确认框")
        compare(findChild(second, "folderError").text, "",
                "旧请求错误不能写进新确认框")
    }

    function test_ok_is_disabled_while_an_action_is_running() {
        backend.fileActionIds = ["id-1"]
        var dialog = startRename()
        var ok = findChild(dialog, "dialogFooterOk")
        compare(ok.enabled, false, "请求进行中：确定置灰，防连点")
    }

    function test_unrelated_action_does_not_disable_this_dialog() {
        backend.fileActionIds = ["other-id"]
        var dialog = startRename()
        var ok = findChild(dialog, "dialogFooterOk")
        compare(ok.enabled, true, "别的条目在写不应禁用当前重命名确认")
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

    function test_dialog_content_does_not_collapse_under_the_footer() {
        // 回归：KDialog 里曾用一个普通子项 `KTheme { id: theme }` 取色。Dialog 的
        // 默认 contentItem 只在「恰好一个子项」时认内容，多一个不可见子项正文就
        // 塌成 0 高——外壳照常居中，输入框被挤没了，页脚按钮压在它上面
        // （2026-10-29 用户截图：所有弹窗都这样）。
        var dialog = startRename()
        var field = findChild(dialog, "renameNameField")
        verify(field !== null, "重命名输入框存在")
        verify(field.height > 20,
               "输入框不能被正文塌陷挤没（实际高 " + field.height + "）")
        var ci = dialog.contentItem
        verify(ci !== null, "弹窗有正文容器")
        var contentBottom = ci.y + ci.height
        verify(dialog.footer.y >= contentBottom,
               "页脚不能压在正文上（正文底 " + contentBottom
               + "，页脚顶 " + dialog.footer.y + "）")

        var ok = findChild(dialog.footer, "dialogFooterOk")
        verify(ok !== null, "页脚有确定按钮")
        var fieldBottom = field.mapToItem(dialog.background, 0, field.height).y
        var okTop = ok.mapToItem(dialog.background, 0, 0).y
        verify(fieldBottom <= okTop,
               "确定按钮不能压住输入框（输入框底 " + fieldBottom
               + "，按钮顶 " + okTop + "）")
    }

    function test_default_dialog_spacing_keeps_title_and_content_apart() {
        var dialog = startRename()
        compare(dialog.spacing, 12, "KDialog 默认标题 / 正文间隔应为 12px")

        var headerBottom = dialog.header.mapToItem(
                    dialog.background, 0, dialog.header.height).y
        var contentTop = dialog.contentItem.mapToItem(
                    dialog.background, 0, 0).y
        compare(contentTop - headerBottom, dialog.spacing,
                "标题和正文之间只保留一个 Dialog.spacing")
    }

    function test_batch_delete_defaults_to_trash_and_closes_immediately() {
        menu.openDelete({ id: "id-1", name: "a.mkv", kind: "file" },
                        ["id-1", "id-2"])
        wait(60)
        var dialog = findChild(menu, "deleteDialog")
        verify(dialog !== null && dialog.opened, "删除确认框打开了")
        verify(findChild(dialog, "deleteMessage").text.indexOf("2 项") >= 0,
               "多选要显示条数")
        var check = findChild(dialog, "deletePermanentCheck")
        verify(check !== null && !check.checked, "默认不彻底删除")

        mouseClick(findChild(dialog, "dialogFooterOk"))
        wait(40)
        compare(backend.lastTrashIds.length, 2, "默认走回收站，批量 id 一次交给后端")
        compare(backend.lastDeleteIds.length, 0, "默认不调彻底删除")
        verify(!dialog.opened, "条目已从列表移除，确认框立即关闭")

        backend.finish("trash", false, "部分条目失败")
        wait(40)
        verify(!dialog.opened, "失败由底栏提示，不能把确认框重新弹出")
    }

    function test_permanent_delete_requires_the_checkbox() {
        menu.openDelete({ id: "id-1", name: "a.mkv", kind: "file" },
                        ["id-1", "id-2"])
        wait(60)
        var dialog = findChild(menu, "deleteDialog")
        var check = findChild(dialog, "deletePermanentCheck")
        var ok = findChild(dialog, "dialogFooterOk")
        check.checked = true
        wait(20)
        compare(ok.text, qsTr("彻底删除"), "勾选后确定按钮改为彻底删除")
        verify(findChild(dialog, "deleteMessage").text.indexOf("无法恢复") >= 0,
               "彻底删除要写明不可恢复")

        mouseClick(ok)
        wait(40)
        compare(backend.lastDeleteIds.length, 2, "勾选后走彻底删除")
        compare(backend.lastTrashIds.length, 0, "不经过回收站")
        verify(!dialog.opened, "请求被接受后立即关窗")

        backend.finish("delete", true, "已彻底删除 2 项")
        wait(40)
        verify(!dialog.opened, "成功后保持关闭")
    }

    function test_permanent_checkbox_resets_each_open() {
        menu.openDelete({ id: "id-1", name: "a.mkv", kind: "file" }, ["id-1"])
        wait(60)
        var dialog = findChild(menu, "deleteDialog")
        var check = findChild(dialog, "deletePermanentCheck")
        check.checked = true
        wait(20)
        dialog.close()
        wait(20)

        menu.openDelete({ id: "id-1", name: "a.mkv", kind: "file" }, ["id-1"])
        wait(60)
        verify(!check.checked, "每次打开都回到默认：移到回收站")
    }
}
