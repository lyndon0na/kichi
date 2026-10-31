import QtQuick
import QtTest
import ".."

// 回收站表格与危险操作的组件测试。
// 重点锁住：批量选择、还原调用、彻底删除必须二次确认，失败时弹窗不消失。
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
        property string statusMessage: ""
        property string contentView: "list"

        property int refreshCount: 0
        property var lastUntrashIds: []
        property var lastDeleteIds: []
        signal changed()
        signal fileActionDone(string action, bool ok, string message)

        function refreshTrash() {
            refreshCount += 1
        }

        function setContentView(view) {
            contentView = view === "grid" ? "grid" : "list"
            changed()
        }

        function untrashEntries(ids) {
            lastUntrashIds = ids.slice()
            fileActionRunning = true
        }

        function deleteTrashEntries(ids) {
            lastDeleteIds = ids.slice()
            fileActionRunning = true
        }

        function finish(action, ok, message) {
            fileActionRunning = false
            fileActionDone(action, ok, message)
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
        backend.statusMessage = ""
        backend.contentView = "list"
        backend.refreshCount = 0
        backend.lastUntrashIds = []
        backend.lastDeleteIds = []
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

    function test_permanent_delete_requires_confirmation() {
        publish([entry("a", false), entry("b", false)])
        selection.selectAll(backend.trashFiles)

        mouseClick(findChild(header, "trashDeleteButton"))
        wait(40)
        compare(backend.lastDeleteIds.length, 0, "确认之前不能请求彻底删除")

        var dialog = findChild(page, "trashDeleteDialog")
        verify(dialog !== null && dialog.opened)
        var message = findChild(dialog, "trashDeleteMessage")
        verify(message.text.indexOf("2 项") >= 0, message.text)
        verify(message.text.indexOf("无法恢复") >= 0, message.text)

        mouseClick(findChild(dialog, "dialogFooterOk"))
        wait(20)
        compare(backend.lastDeleteIds.length, 2)
        verify(dialog.opened, "请求还没回来，弹窗不关")
        backend.finish("trashDelete", true, "已彻底删除 2 项")
        tryCompare(dialog, "opened", false)
        compare(selection.count, 0, "成功后清空选择")
    }

    function test_delete_key_only_opens_confirmation() {
        publish([entry("a", false)])
        selection.selectAll(backend.trashFiles)
        list.forceActiveFocus()
        keyClick(Qt.Key_Delete)
        wait(40)
        compare(backend.lastDeleteIds.length, 0)
        var dialog = findChild(page, "trashDeleteDialog")
        verify(dialog !== null && dialog.opened)
    }

    function test_failed_delete_stays_open_with_reason() {
        publish([entry("a", false)])
        selection.selectAll(backend.trashFiles)
        mouseClick(findChild(header, "trashDeleteButton"))
        wait(40)

        var dialog = findChild(page, "trashDeleteDialog")
        mouseClick(findChild(dialog, "dialogFooterOk"))
        wait(20)
        backend.finish("trashDelete", false, "file_not_found")
        wait(20)
        verify(dialog.opened)
        var error = findChild(dialog, "trashDeleteError")
        verify(error.visible)
        compare(error.text, "file_not_found")
    }

    function test_header_puts_text_refresh_last_and_has_no_clear_entry() {
        publish([entry("a", false), entry("b", true), entry("c", false)])
        var refresh = findChild(header, "trashRefreshButton")
        var deleteButton = findChild(header, "trashDeleteButton")
        var toggle = findChild(header, "viewToggleButton")
        verify(refresh !== null && refresh.visible && refresh.enabled)
        compare(refresh.text, "刷新")
        verify(refresh.x > deleteButton.x, "刷新按钮应在最右侧")
        verify(toggle !== null && toggle.x < refresh.x,
               "视图切换应排在刷新之前")
        verify(findChild(header, "trashClearButton") === null, "回收站不再提供清空入口")

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
