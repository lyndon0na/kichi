import QtQuick
import QtTest
import ".."

// 回收站表格与危险操作的组件测试。
// 重点锁住：批量选择、还原、选中项彻底删除和整库清空的确认与异步边界。
TestCase {
    name: "TrashPage"
    when: windowShown
    width: 980
    height: 640

    QtObject {
        id: backend

        property var trashFiles: []
        property bool trashLoading: false
        property bool trashLoaded: false
        property string trashError: ""
        property int trashRevision: 0
        property bool filesStale: false
        property bool fileActionRunning: false
        property var fileActionIds: []
        property bool trashActionRunning: false
        property bool trashClearing: false
        property string statusMessage: ""
        property string contentView: "list"

        property int refreshCount: 0
        property var lastUntrashIds: []
        property var lastDeleteIds: []
        property int clearCount: 0
        property var deleteSnapshot: []
        property var clearSnapshot: []
        signal changed()
        signal fileActionDone(string action, bool ok, string message, var ids)

        function refreshTrash() {
            refreshCount += 1
        }

        function setContentView(view) {
            contentView = view === "grid" ? "grid" : "list"
            changed()
        }

        function untrashEntries(ids) {
            lastUntrashIds = ids.slice()
            trashActionRunning = true
            fileActionIds = ids.slice()
            changed()
        }

        function deleteTrashEntries(ids) {
            lastDeleteIds = ids.slice()
            deleteSnapshot = trashFiles.slice()
            var wanted = {}
            for (var i = 0; i < ids.length; ++i)
                wanted[ids[i]] = true
            var kept = []
            for (var j = 0; j < trashFiles.length; ++j) {
                if (!wanted[trashFiles[j].id])
                    kept.push(trashFiles[j])
            }
            trashFiles = kept
            trashRevision += 1
            trashActionRunning = true
            fileActionIds = ids.slice()
            changed()
            return true
        }

        function clearTrash() {
            clearCount += 1
            clearSnapshot = trashFiles.slice()
            trashFiles = []
            trashRevision += 1
            trashActionRunning = true
            trashClearing = true
            fileActionIds = []
            changed()
            return true
        }

        function finish(action, ok, message) {
            fileActionRunning = false
            trashActionRunning = false
            trashClearing = false
            if (!ok) {
                if (action === "trashDelete" && deleteSnapshot.length > 0) {
                    trashFiles = deleteSnapshot.slice()
                    trashRevision += 1
                    changed()
                } else if (action === "trashClear" && clearSnapshot.length > 0) {
                    trashFiles = clearSnapshot.slice()
                    trashRevision += 1
                    changed()
                }
                statusMessage = message
            } else {
                statusMessage = ""
            }
            deleteSnapshot = []
            clearSnapshot = []
            const ids = action === "trashDelete"
                        ? lastDeleteIds.slice()
                        : (action === "untrash" ? lastUntrashIds.slice() : [])
            fileActionIds = []
            fileActionDone(action, ok, message, ids)
            changed()
        }
    }

    Component {
        id: pageComponent

        TrashPage {
            width: 980
            height: 640
        }
    }

    Component {
        id: headerComponent
        TrashHeaderControls {}
    }

    property var page: null
    property var header: null
    property var list: null
    property var selection: null

    function init() {
        backend.trashFiles = []
        backend.trashLoading = false
        backend.trashLoaded = false
        backend.trashError = ""
        backend.trashRevision = 0
        backend.filesStale = false
        backend.fileActionRunning = false
        backend.fileActionIds = []
        backend.trashActionRunning = false
        backend.trashClearing = false
        backend.statusMessage = ""
        backend.contentView = "list"
        backend.refreshCount = 0
        backend.lastUntrashIds = []
        backend.lastDeleteIds = []
        backend.clearCount = 0
        backend.deleteSnapshot = []
        backend.clearSnapshot = []
        page = pageComponent.createObject(parent, {
            visible: true,
            backend: backend
        })
        verify(page !== null, "TrashPage 创建失败")
        // 页头在 main.qml 的标题行里；单独摆到页面右侧，避免盖住左侧列表的点击区。
        header = headerComponent.createObject(parent, {
            x: 520,
            y: 0,
            width: 460,
            target: page
        })
        verify(header !== null, "TrashHeaderControls 创建失败")
        list = findChild(page, "trashList")
        selection = findChild(page, "trashSelection")
        verify(list !== null, "缺少回收站列表")
        verify(selection !== null, "缺少回收站选择器")
        wait(40)
        backend.refreshCount = 0
    }

    function test_first_loading_uses_skeleton_then_restores_spinner() {
        backend.trashFiles = []
        backend.trashLoaded = false
        backend.trashLoading = true
        const freshPage = pageComponent.createObject(parent, {
            visible: false,
            backend: backend
        })
        verify(freshPage !== null)

        freshPage.visible = true
        wait(30)
        const skeleton = findChild(freshPage, "pageLoadingSkeleton")
        const spinner = findChild(freshPage, "trashLoadingIndicator")
        verify(skeleton !== null && spinner !== null)
        compare(skeleton.visible, true, "首次读取显示骨架")
        compare(spinner.visible, false, "首次读取不叠原来的转圈")

        backend.trashLoading = false
        backend.trashLoaded = true
        backend.trashFiles = [{
            id: "trash-first",
            name: "first.txt",
            kind: "file",
            sizeText: "1 KB",
            deletedText: "2026-11-02 10:00"
        }]
        backend.trashRevision += 1
        backend.changed()
        wait(30)
        compare(skeleton.visible, false, "数据到达后收起骨架")

        backend.trashFiles = []
        backend.trashLoaded = false
        backend.trashLoading = true
        backend.trashRevision += 1
        backend.changed()
        wait(30)
        compare(skeleton.visible, false, "第二次读取不再显示骨架")
        compare(spinner.visible, true, "第二次读取恢复原加载态")

        freshPage.destroy()
    }

    function cleanup() {
        if (page)
            page.destroy()
        if (header)
            header.destroy()
        page = null
        header = null
        list = null
        selection = null
    }

    function entry(id, folder) {
        return {
            id: id,
            name: id + (folder ? " 文件夹" : ".mkv"),
            kind: folder ? "folder" : "file",
            sizeText: folder ? "—" : "1.0 GB",
            deletedText: "2026-10-28 12:34"
        }
    }

    function publish(entries) {
        backend.trashFiles = entries
        backend.trashLoaded = true
        backend.trashRevision += 1
        backend.changed()
        wait(20)
    }

    function test_becoming_visible_refreshes_and_shows_empty_state() {
        page.visible = false
        wait(10)
        backend.refreshCount = 0
        page.visible = true
        tryCompare(backend, "refreshCount", 1)

        backend.trashLoaded = true
        backend.changed()
        var empty = findChild(page, "trashEmptyHint")
        verify(empty !== null && empty.visible)
    }

    function test_rows_follow_backend_order() {
        publish([entry("new", false), entry("old", true)])
        compare(list.count, 2)
        compare(list.itemAtIndex(0).entry.id, "new")
        compare(list.itemAtIndex(1).entry.id, "old")
    }

    function test_rows_use_the_shared_flat_table_metrics() {
        publish([entry("a", false)])
        compare(list.spacing, 0)
        var row = list.itemAtIndex(0)
        verify(row !== null)
        var density = findChild(page, "fileListDensity")
        verify(density !== null)
        compare(row.implicitHeight, density.rowHeight)
        compare(row.background.radius, 0)
        compare(findChild(row, "thumbnailSlot").width, 36)
        compare(findChild(row, "fileThumbnail").width, 36)
        compare(findChild(row, "fileThumbnail").height, 24)
        verify(findChild(row, "trashRowDivider") !== null)
    }

    function test_grid_mode_uses_the_shared_cards_and_supports_selection() {
        publish([entry("a", false), entry("b", true)])
        const list = findChild(page, "trashList")
        const grid = findChild(page, "trashGrid")
        verify(list !== null && grid !== null)
        compare(list.visible, true)
        compare(grid.visible, false)

        page.setContentView("grid")
        tryCompare(backend, "contentView", "grid")
        compare(list.visible, false)
        compare(grid.visible, true)
        compare(grid.count, 2)

        const card = grid.itemAtIndex(1)
        verify(card !== null)
        compare(card.activationEnabled, false, "回收站网格双击不能执行动作")
        const detail = findChild(card, "gridDetail")
        verify(detail !== null)
        verify(detail.text.indexOf("26-10-28 12:34") >= 0)

        mouseClick(card, 20, 20)
        compare(selection.count, 1)
        verify(selection.isSelected("b"))
    }

    function test_list_and_grid_keep_independent_scroll_positions() {
        const rows = []
        for (let i = 0; i < 40; ++i)
            rows.push(entry("item-" + i, false))
        publish(rows)

        const list = findChild(page, "trashList")
        verify(list !== null)
        list.contentY = 144
        wait(30)

        page.setContentView("grid")
        const grid = findChild(page, "trashGrid")
        verify(grid !== null)
        tryCompare(grid, "visible", true)
        grid.contentY = 96
        wait(30)

        page.setContentView("list")
        tryCompare(backend, "contentView", "list")
        tryCompare(list, "contentY", 144)
    }

    function test_context_menu_uses_row_window_coordinates() {
        publish([entry("a", false)])
        var row = list.itemAtIndex(0)
        verify(row !== null)
        var local = Qt.point(24, 13)
        var expected = row.mapToItem(null, local.x, local.y)
        row.contextMenu(local)
        wait(40)

        var menu = findChild(page, "trashContextMenu")
        verify(menu !== null && menu.visible)
        var deleteItem = findChild(menu, "trashDeleteMenuItem")
        verify(deleteItem !== null && deleteItem.enabled,
               "右键菜单仍应提供选中项彻底删除")
        verify(Math.abs(menu.x - expected.x) <= 2,
               "菜单 x 错位：实际 " + menu.x + "，期望 " + expected.x)
        verify(Math.abs(menu.y - expected.y) <= 2,
               "菜单 y 错位：实际 " + menu.y + "，期望 " + expected.y)
        menu.close()
    }

    function test_select_all_restores_every_selected_id() {
        publish([entry("a", false), entry("b", true), entry("c", false)])
        compare(findChild(header, "trashSelectionSummary").text, "")
        selection.selectAll(backend.trashFiles)
        compare(selection.count, 3)

        var summary = findChild(header, "trashSelectionSummary")
        verify(summary.text.indexOf("3") >= 0, summary.text)

        mouseClick(findChild(header, "trashRestoreButton"))
        wait(20)
        compare(backend.lastUntrashIds.length, 3)
        verify(backend.lastUntrashIds.indexOf("a") >= 0)
        verify(backend.lastUntrashIds.indexOf("b") >= 0)
        verify(backend.lastUntrashIds.indexOf("c") >= 0)
    }

    function test_unrelated_action_keeps_other_trash_rows_operable() {
        publish([entry("a", false), entry("b", false)])
        selection.selectOnly("b")
        backend.fileActionIds = ["a"]
        backend.trashActionRunning = true
        backend.changed()
        wait(20)

        var restore = findChild(header, "trashRestoreButton")
        var clear = findChild(header, "trashClearButton")
        var refresh = findChild(header, "trashRefreshButton")
        verify(restore.enabled, "另一个条目在删，仍可还原当前选中的条目")
        compare(clear.enabled, false, "回收站有写操作时不能同时清空")
        compare(refresh.enabled, false, "写操作期间回收站刷新要等收尾")

        backend.fileActionIds = ["b"]
        backend.changed()
        wait(20)
        compare(restore.enabled, false, "同一目标在写时不能重复操作")
    }

    function test_clear_trash_requires_confirmation_and_returns_immediately() {
        publish([entry("a", false), entry("b", false)])
        selection.selectAll(backend.trashFiles)

        var clearButton = findChild(header, "trashClearButton")
        verify(clearButton !== null && clearButton.enabled)
        mouseClick(clearButton)
        wait(40)
        compare(backend.clearCount, 0, "确认之前不能请求清空")

        var dialog = findChild(page, "trashClearDialog")
        verify(dialog !== null && dialog.opened)
        var message = findChild(dialog, "trashClearMessage")
        verify(message.text.indexOf("2 项") >= 0, message.text)
        verify(message.text.indexOf("无法恢复") >= 0, message.text)

        mouseClick(findChild(dialog, "dialogFooterOk"))
        wait(20)
        compare(backend.clearCount, 1)
        compare(dialog.opened, false, "请求发出后立即收起，不阻塞页面")
        compare(backend.trashFiles.length, 0, "确认后先清空本地列表")
        compare(list.count, 0)
        compare(clearButton.enabled, false, "后台请求在途时按钮禁用")
        compare(selection.count, 2, "异步清空期间保留选择，失败可回滚")

        backend.finish("trashClear", true, "回收站已清空（共 2 项）")
        wait(20)
        compare(selection.count, 0, "成功后清空选择")
        compare(clearButton.enabled, false, "清空后回收站仍为空")
    }

    function test_permanent_delete_requires_confirmation() {
        publish([entry("a", false), entry("b", false)])
        selection.selectAll(backend.trashFiles)
        list.forceActiveFocus()
        keyClick(Qt.Key_Delete)
        wait(40)
        compare(backend.clearCount, 0)
        compare(backend.lastDeleteIds.length, 0, "确认之前不能请求彻底删除")

        var dialog = findChild(page, "trashDeleteDialog")
        verify(dialog !== null && dialog.opened)
        var message = findChild(dialog, "trashDeleteMessage")
        verify(message.text.indexOf("2 项") >= 0, message.text)
        verify(message.text.indexOf("无法恢复") >= 0, message.text)

        mouseClick(findChild(dialog, "dialogFooterOk"))
        wait(20)
        compare(backend.lastDeleteIds.length, 2)
        compare(dialog.opened, false, "请求发出后立即收起，不阻塞页面")
        compare(backend.trashFiles.length, 0, "确认后先移除选中的行")
        compare(list.count, 0)
        compare(selection.count, 2, "乐观移除期间保留选择，失败可回滚")
        compare(backend.trashActionRunning, true)
        backend.finish("trashDelete", true, "已彻底删除 2 项")
        wait(20)
        compare(selection.count, 0, "成功后清空选择")
    }

    function test_failed_delete_restores_rows_without_reopening_dialog() {
        publish([entry("a", false)])
        selection.selectAll(backend.trashFiles)
        list.forceActiveFocus()
        keyClick(Qt.Key_Delete)
        wait(40)

        var dialog = findChild(page, "trashDeleteDialog")
        mouseClick(findChild(dialog, "dialogFooterOk"))
        wait(20)
        compare(dialog.opened, false)
        compare(backend.trashFiles.length, 0)
        backend.finish("trashDelete", false, "file_not_found")
        wait(20)
        compare(dialog.opened, false, "失败只在底栏提示，不重开确认框")
        compare(backend.trashFiles.length, 1, "失败后恢复本地行")
        compare(list.count, 1)
        compare(list.itemAtIndex(0).entry.id, "a")
        compare(selection.count, 1, "失败回滚后保留原选择")
        compare(backend.statusMessage, "file_not_found")
    }

    function test_failed_clear_restores_rows_without_reopening_dialog() {
        publish([entry("a", false), entry("b", true)])
        selection.selectAll(backend.trashFiles)
        mouseClick(findChild(header, "trashClearButton"))
        wait(20)

        var dialog = findChild(page, "trashClearDialog")
        mouseClick(findChild(dialog, "dialogFooterOk"))
        wait(20)
        compare(dialog.opened, false)
        compare(list.count, 0)

        backend.finish("trashClear", false, "network_error")
        wait(20)
        compare(dialog.opened, false, "失败只在底栏提示，不重开确认框")
        compare(backend.trashFiles.length, 2, "失败后恢复整份列表")
        compare(list.count, 2)
        compare(selection.count, 2, "失败回滚后保留原选择")
        compare(backend.statusMessage, "network_error")
    }

    function test_partial_delete_failure_drops_completed_rows_from_selection() {
        publish([entry("a", false), entry("b", false), entry("c", false)])
        selection.selectAll(backend.trashFiles)
        backend.deleteTrashEntries(["a", "b", "c"])
        wait(20)
        compare(selection.count, 3)

        // Rust 先恢复失败批与未发送项，此时锁还没释放，选择必须暂存；
        // fileActionDone 发出时锁已释放，完成删除的行不再属于选择。
        backend.trashFiles = [entry("b", false)]
        backend.trashRevision += 1
        backend.fileActionIds = ["a", "b", "c"]
        backend.changed()
        wait(20)
        compare(selection.count, 3, "回滚尚未收尾时保留原选择")

        backend.trashActionRunning = false
        backend.fileActionIds = []
        backend.fileActionDone("trashDelete", false, "中断：只恢复了部分条目", ["a", "b", "c"])
        backend.changed()
        wait(20)
        compare(selection.count, 1, "只保留仍在列表里的恢复项")
        verify(selection.isSelected("b"))
    }

    function test_clear_button_is_disabled_for_an_empty_trash() {
        backend.trashLoaded = true
        backend.changed()
        wait(20)
        var clearButton = findChild(header, "trashClearButton")
        verify(clearButton !== null)
        compare(clearButton.enabled, false)
    }

    function test_header_puts_text_refresh_last_and_has_no_delete_entry() {
        publish([entry("a", false), entry("b", true), entry("c", false)])
        var refresh = findChild(header, "trashRefreshButton")
        var clearButton = findChild(header, "trashClearButton")
        var toggle = findChild(header, "viewToggleButton")
        verify(refresh !== null && refresh.visible && refresh.enabled)
        verify(clearButton !== null && clearButton.visible && clearButton.enabled)
        compare(refresh.text, "刷新")
        compare(clearButton.text, "清空回收站")
        verify(refresh.x > clearButton.x, "刷新按钮应在最右侧")
        verify(toggle !== null && toggle.x < refresh.x,
               "视图切换应排在刷新之前")
        verify(findChild(header, "trashDeleteButton") === null,
               "回收站不再提供永久删除入口")

        mouseClick(refresh)
        wait(20)
        compare(backend.refreshCount, 1)
    }

    function test_header_refresh_feedback_hides_idle_count() {
        publish([entry("a", false), entry("b", true)])
        var summary = findChild(header, "trashSelectionSummary")
        var refresh = findChild(header, "trashRefreshButton")
        verify(summary !== null && refresh !== null)
        var spinner = findChild(refresh, "buttonBusyIndicator")
        verify(spinner !== null)
        compare(summary.text, "")
        compare(refresh.text, "刷新")
        compare(refresh.enabled, true)
        compare(spinner.running, false)
        var idleWidth = refresh.width

        // 后台校准不改 trashLoading，按钮不闪。
        backend.trashRevision += 1
        backend.changed()
        wait(20)
        compare(summary.text, "")
        compare(refresh.text, "刷新")
        compare(refresh.enabled, true)
        compare(spinner.running, false)

        backend.trashLoading = true
        backend.changed()
        wait(20)
        compare(summary.text, "")
        compare(refresh.text, "刷新")
        compare(refresh.enabled, false)
        compare(spinner.running, true)
        compare(refresh.width, idleWidth)

        backend.trashLoading = false
        backend.changed()
        wait(20)
        compare(summary.text, "")
        compare(refresh.text, "刷新")
        compare(refresh.enabled, true)
        compare(spinner.running, false)
        compare(refresh.width, idleWidth)
    }
}
