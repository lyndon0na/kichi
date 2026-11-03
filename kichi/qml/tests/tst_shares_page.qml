import QtQuick
import QtTest
import ".."

TestCase {
    name: "SharesPage"
    when: windowShown
    width: 900
    height: 650

    QtObject {
        id: backend

        property var shares: []
        property bool sharesLoading: false
        property bool shareActionRunning: false
        property int refreshCalls: 0
        property int revokeCalls: 0
        property var lastRevokeIds: []

        signal changed()
        signal shareActionDone(string action, bool ok, string message)

        function refreshShares() {
            refreshCalls += 1
        }

        function revokeShares(ids) {
            revokeCalls += 1
            lastRevokeIds = ids.slice()
            shareActionRunning = true
        }

        function finishRevoke(ok, message) {
            shareActionRunning = false
            shareActionDone("revoke", ok, message)
        }
    }

    Component {
        id: pageComponent
        SharesPage {}
    }

    Component {
        id: headerComponent
        ShareHeaderControls {}
    }

    property var page: null
    property var header: null
    property var list: page ? findChild(page, "sharesList") : null
    property var model: list ? list.model : null

    function init() {
        page = pageComponent.createObject(parent, {
            width: 900,
            height: 650,
            backend: backend
        })
        verify(page !== null, "SharesPage 创建失败")
        header = headerComponent.createObject(parent, {
            width: 700,
            target: page
        })
        verify(header !== null, "ShareHeaderControls 创建失败")
        list = findChild(page, "sharesList")
        model = list ? list.model : null
        backend.sharesLoading = false
        backend.shareActionRunning = false
        page.active = true
        backend.refreshCalls = 0
        backend.revokeCalls = 0
        backend.lastRevokeIds = []
        push([])
    }

    function cleanup() {
        if (page)
            page.destroy()
        if (header)
            header.destroy()
        page = null
        header = null
        list = null
        model = null
    }

    function share(id, passCode) {
        const code = passCode === undefined ? "p" + id : passCode
        return {
            id: id,
            title: id + " 的分享",
            kind: "",
            url: "https://mypikpak.com/s/" + id,
            passCode: code,
            hasPassword: code.length > 0,
            accessText: code.length > 0 ? "密码保护" : "公开",
            expirationText: "7 天",
            createdText: "2026-10-29 10:00",
            statsText: "文件 2 · 浏览 3 · 转存 1",
            statusText: "有效",
            statusTone: "success"
        }
    }

    function push(shares) {
        backend.shares = shares
        backend.changed()
    }

    function test_rows_follow_backend_order() {
        push([share("a"), share("b")])
        compare(model.count, 2)
        compare(model.get(0).key, "a")

        push([share("b")])
        compare(model.count, 1)
        compare(model.get(0).key, "b")
    }

    function test_scroll_stops_at_bounds() {
        compare(list.boundsBehavior, Flickable.StopAtBounds)
    }

    function test_table_uses_uniform_flat_density() {
        push([share("a")])
        waitForRendering(list)

        var header = findChild(page, "sharesTableHeader")
        verify(header)
        compare(header.implicitHeight, 34)
        compare(list.spacing, 0)
        compare(findChild(page, "sharesTableLayout").spacing, 4)

        var row = list.itemAtIndex(0)
        verify(row)
        compare(row.implicitHeight, 48)
        compare(row.background.radius, 0)
    }

    function test_rows_reorder_when_a_share_is_removed() {
        push([share("a"), share("b"), share("c")])
        push([share("c"), share("b")])
        compare(model.count, 2)
        compare(model.get(0).key, "c")
        compare(model.get(1).key, "b")
    }

    function test_activation_refreshes_share_list() {
        page.active = false
        backend.refreshCalls = 0
        page.active = true
        tryCompare(backend, "refreshCalls", 1)
        page.active = false
        page.active = true
        tryCompare(backend, "refreshCalls", 2)
        mouseClick(findChild(header, "refreshShares"))
        tryCompare(backend, "refreshCalls", 3)
    }

    function test_first_loading_uses_skeleton_then_restores_spinner() {
        backend.shares = []
        backend.sharesLoading = true
        page.active = true
        wait(30)

        const skeleton = findChild(page, "pageLoadingSkeleton")
        const loadingState = findChild(page, "sharesLoadingState")
        verify(skeleton !== null && loadingState !== null)
        compare(skeleton.visible, true, "首次读取显示骨架")
        compare(loadingState.visible, false, "首次读取不叠原来的转圈")

        backend.sharesLoading = false
        push([share("a")])
        wait(30)
        compare(skeleton.visible, false, "数据到达后收起骨架")

        push([])
        backend.sharesLoading = true
        backend.changed()
        wait(30)
        compare(skeleton.visible, false, "第二次读取不再显示骨架")
        compare(loadingState.visible, true, "第二次读取恢复原加载态")
    }

    function test_header_refresh_feedback_hides_idle_count() {
        push([share("a"), share("b")])
        waitForRendering(list)
        var status = findChild(header, "shareHeaderStatus")
        var refresh = findChild(header, "refreshShares")
        verify(status !== null && refresh !== null)
        var spinner = findChild(refresh, "buttonBusyIndicator")
        verify(spinner !== null)
        compare(status.text, "")
        compare(refresh.text, "刷新")
        compare(refresh.enabled, true)
        compare(spinner.running, false)
        var idleWidth = refresh.width

        backend.sharesLoading = true
        backend.changed()
        wait(20)
        compare(status.text, "")
        compare(refresh.text, "刷新")
        compare(refresh.enabled, false)
        compare(spinner.running, true)
        compare(refresh.width, idleWidth)

        backend.sharesLoading = false
        backend.changed()
        wait(20)
        compare(status.text, "")
        compare(refresh.text, "刷新")
        compare(refresh.enabled, true)
        compare(spinner.running, false)
        compare(refresh.width, idleWidth)
    }

    function test_table_has_file_icon_and_selection_actions_without_checkboxes() {
        push([share("a")])
        waitForRendering(list)

        verify(findChild(page, "sharesTableHeader"))
        verify(findChild(header, "revokeSelectedShares"))
        verify(findChild(header, "revokeSelectedShares").enabled === false)

        const row = list.itemAtIndex(0)
        verify(row)
        verify(findChild(row, "shareTypeIcon"))
        verify(findChild(row, "shareSelectCheckBox") === null)
    }

    function test_shared_folder_uses_api_kind_for_icon() {
        var folder = share("folder")
        folder.title = "电影合集"
        folder.kind = "folder"
        push([folder])
        waitForRendering(list)

        const row = list.itemAtIndex(0)
        const typeIcon = findChild(row, "shareTypeIcon")
        verify(typeIcon)
        compare(typeIcon.iconName, "folder")
    }

    function test_missing_kind_still_uses_extension_for_icon() {
        var video = share("video")
        video.title = "电影.mkv"
        video.kind = ""
        push([video])
        waitForRendering(list)

        const row = list.itemAtIndex(0)
        const typeIcon = findChild(row, "shareTypeIcon")
        verify(typeIcon)
        compare(typeIcon.iconName, "video")
    }

    function test_row_hover_stays_visible_across_content() {
        push([share("a")])
        waitForRendering(list)

        const row = list.itemAtIndex(0)
        verify(row)
        const points = [[5, 5], [120, 20], [360, 20],
                        [row.width - 48, row.height / 2]]
        for (let i = 0; i < points.length; i++) {
            mouseMove(row, points[i][0], points[i][1])
            wait(20)
            verify(row.hovered, "row hover lost at " + points[i].join(","))
            verify(row.background.color.a > 0,
                   "hover background lost at " + points[i].join(","))
        }
    }

    function test_copy_button_copies_link_and_password_together() {
        push([share("a")])
        waitForRendering(list)

        const row = list.itemAtIndex(0)
        verify(row)
        const button = findChild(row, "shareCopyButton")
        const editor = findChild(page, "clipboardEditor")
        verify(button)
        verify(editor)

        mouseClick(button)
        compare(editor.text, "a 的分享\nhttps://mypikpak.com/s/a\n访问密码：pa")
        compare(page.copyNotice, "链接和密码已复制")
        compare(page.selectedCount, 0, "点行内复制按钮不应顺带选中整行")
    }

    function test_copy_without_password_reports_link_only() {
        push([share("a", "")])
        waitForRendering(list)

        const row = list.itemAtIndex(0)
        verify(row)
        const button = findChild(row, "shareCopyButton")
        const editor = findChild(page, "clipboardEditor")
        verify(button)
        verify(editor)

        mouseClick(button)
        compare(editor.text, "a 的分享\nhttps://mypikpak.com/s/a")
        compare(page.copyNotice, "链接已复制")
    }

    function test_ctrl_shift_click_selects_rows() {
        push([share("a"), share("b"), share("c")])
        waitForRendering(list)

        mouseClick(list.itemAtIndex(0), 20, 20, Qt.LeftButton)
        compare(page.selectedIdsInOrder().join(","), "a")
        mouseClick(list.itemAtIndex(2), 20, 20, Qt.LeftButton, Qt.ControlModifier)
        compare(page.selectedIdsInOrder().join(","), "a,c")
        mouseClick(list.itemAtIndex(1), 20, 20, Qt.LeftButton, Qt.ShiftModifier)
        compare(page.selectedIdsInOrder().join(","), "b,c")
    }

    function test_ctrl_a_selects_all_shares() {
        push([share("a"), share("b"), share("c")])
        waitForRendering(list)
        compare(findChild(header, "shareHeaderStatus").text, "")
        mouseClick(list.itemAtIndex(0), 20, 20)
        compare(page.selectedIdsInOrder().join(","), "a")
        keyClick(Qt.Key_A, Qt.ControlModifier)
        wait(20)
        compare(page.selectedIdsInOrder().join(","), "a,b,c")
        compare(findChild(header, "shareHeaderStatus").text, "已选择 3 个分享")
    }

    function test_right_click_narrows_to_unselected_row() {
        push([share("a"), share("b"), share("c")])
        waitForRendering(list)
        page.selectRow("a", 0, Qt.NoModifier)
        page.selectRow("c", 2, Qt.ControlModifier)

        page.openRowMenu("b", Qt.point(100, 100))
        compare(page.selectedIdsInOrder().join(","), "b")
        compare(page.shareMenu.targetIds.join(","), "b")
        page.shareMenu.menu.close()
    }

    function test_row_right_click_keeps_a_selected_multi_selection() {
        push([share("a"), share("b"), share("c")])
        waitForRendering(list)
        mouseClick(list.itemAtIndex(0), 20, 20, Qt.LeftButton)
        mouseClick(list.itemAtIndex(2), 20, 20, Qt.LeftButton, Qt.ControlModifier)

        mouseClick(list.itemAtIndex(0), 20, 20, Qt.RightButton)
        compare(page.selectedIdsInOrder().join(","), "a,c")
        compare(page.shareMenu.targetIds.join(","), "a,c")
        page.shareMenu.menu.close()
    }

    function test_context_menu_has_only_copy_and_revoke() {
        push([share("a")])
        waitForRendering(list)
        page.openRowMenu("a", Qt.point(100, 100))

        verify(findChild(page.shareMenu.menu, "shareCopyLinkItem"))
        verify(findChild(page.shareMenu.menu, "shareRevokeItem"))
        verify(findChild(page.shareMenu.menu, "shareExpirationItem") === null)
        page.shareMenu.menu.close()
    }

    function test_batch_copy_keeps_table_order_and_password() {
        push([share("a"), share("b"), share("c")])
        waitForRendering(list)

        page.copyShares(["c", "a"])
        const editor = findChild(page, "clipboardEditor")
        verify(editor)
        compare(editor.text,
                "a 的分享\nhttps://mypikpak.com/s/a\n访问密码：pa\n\n"
                + "c 的分享\nhttps://mypikpak.com/s/c\n访问密码：pc")
        compare(page.copyNotice, "已复制 2 个分享链接")
    }

    function test_revoke_selected_uses_confirmation_and_clears_after_success() {
        push([share("a"), share("b")])
        waitForRendering(list)
        page.selectRow("a", 0, Qt.NoModifier)
        page.selectRow("b", 1, Qt.ControlModifier)

        const revokeButton = findChild(header, "revokeSelectedShares")
        verify(revokeButton && revokeButton.enabled)
        mouseClick(revokeButton)
        verify(page.revokeDialog.dialog.opened)
        compare(backend.revokeCalls, 0)

        const ok = findChild(page.revokeDialog.dialog, "dialogFooterOk")
        verify(ok)
        mouseClick(ok)
        compare(backend.revokeCalls, 1)
        compare(backend.lastRevokeIds.join(","), "a,b")

        backend.finishRevoke(true, "已取消")
        tryCompare(page.revokeDialog.dialog, "opened", false)
        compare(page.selectedCount, 0)
        compare(page.copyNotice, "已取消 2 个分享")
    }

    function test_revoke_failure_keeps_confirmation_open() {
        push([share("a")])
        waitForRendering(list)
        page.selectRow("a", 0, Qt.NoModifier)
        page.openRevoke(page.selectedIdsInOrder())

        const ok = findChild(page.revokeDialog.dialog, "dialogFooterOk")
        verify(ok)
        mouseClick(ok)
        backend.finishRevoke(false, "取消分享失败")

        compare(page.revokeDialog.dialog.opened, true)
        compare(findChild(page.revokeDialog.dialog, "shareRevokeError").text,
                "取消分享失败")
        page.revokeDialog.dialog.close()
    }

    function test_inactive_page_releases_rows_and_restores_selection() {
        push([share("a"), share("b"), share("c")])
        compare(model.count, 3)
        page.selectRow("b", 1, Qt.NoModifier)
        compare(page.selectedCount, 1)

        page.active = false
        tryCompare(model, "count", 0)
        push([share("d")])
        compare(model.count, 0, "隐藏页收到后端更新时不能重新建行")

        page.active = true
        tryCompare(model, "count", 1)
        compare(page.selectedCount, 0, "刷新后已不存在的选中项应被清理")
    }
}
