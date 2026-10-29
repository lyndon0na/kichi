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
        property var revokedIds: []

        signal changed()
        signal shareActionDone(string action, bool ok, string message)

        function refreshShares() {
            refreshCalls += 1
        }

        function revokeShares(ids) {
            revokedIds = ids.slice()
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

    property var page: null
    property var list: page ? findChild(page, "sharesList") : null
    property var model: list ? list.model : null
    property var revokeDialog: page ? findChild(page, "revokeSharesDialog") : null

    function init() {
        page = pageComponent.createObject(parent, {
            width: 900,
            height: 650,
            backend: backend
        })
        verify(page !== null, "SharesPage 创建失败")
        list = findChild(page, "sharesList")
        model = list ? list.model : null
        revokeDialog = findChild(page, "revokeSharesDialog")
        page.active = false
        page.clearSelection()
        backend.sharesLoading = false
        backend.shareActionRunning = false
        backend.refreshCalls = 0
        backend.revokedIds = []
        if (revokeDialog.visible)
            revokeDialog.close()
        push([])
    }

    function cleanup() {
        if (page)
            page.destroy()
        page = null
        list = null
        model = null
        revokeDialog = null
    }

    function share(id) {
        return {
            id: id,
            title: id + " 的分享",
            url: "https://mypikpak.com/s/" + id,
            passCode: "p" + id,
            hasPassword: true,
            accessText: "密码保护",
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

    function test_rows_follow_backend_and_prune_selection() {
        push([share("a"), share("b")])
        compare(model.count, 2)
        page.selectIds(["a", "b"])
        compare(page.selectedCount, 2)

        push([share("b")])
        compare(model.count, 1)
        compare(model.get(0).key, "b")
        compare(page.selectedCount, 1)
        compare(page.isSelected("b"), true)
    }

    function test_rows_reorder_when_a_share_is_removed_and_titles_change() {
        push([share("a"), share("b"), share("c")])
        push([share("c"), share("b")])
        compare(model.count, 2)
        compare(model.get(0).key, "c")
        compare(model.get(1).key, "b")
    }

    function test_activation_refreshes_share_list() {
        page.active = true
        tryCompare(backend, "refreshCalls", 1)
        page.active = false
        page.active = true
        tryCompare(backend, "refreshCalls", 2)
    }

    function test_batch_revoke_uses_every_selected_id() {
        push([share("a"), share("b"), share("c")])
        page.selectIds(["a", "c"])
        page.openRevoke(page.selectedIds())
        tryCompare(revokeDialog, "visible", true)
        page.submitRevoke()

        compare(backend.revokedIds.length, 2)
        verify(backend.revokedIds.indexOf("a") >= 0)
        verify(backend.revokedIds.indexOf("c") >= 0)

        backend.finishRevoke(true, "done")
        tryCompare(revokeDialog, "visible", false)
        compare(page.selectedCount, 0)
    }

    function test_checkbox_click_and_external_selection_stay_in_sync() {
        push([share("a"), share("b")])
        waitForRendering(list)

        const row = list.itemAtIndex(0)
        verify(row)
        const checkbox = findChild(row, "shareSelectCheckBox")
        verify(checkbox)

        let toggles = 0
        checkbox.toggled.connect(function () { toggles += 1 })
        mouseClick(checkbox)
        wait(50)
        compare(toggles, 1, "复选框应收到点击（size=" + checkbox.width + "×"
                + checkbox.height + "，visible=" + checkbox.visible
                + "，row=" + row.width + "×" + row.height
                + "，list=" + list.width + "×" + list.height
                + "，pageVisible=" + page.visible + "）")
        tryCompare(page, "selectedCount", 1)
        page.clearSelection()
        tryCompare(checkbox, "checked", false)
        page.toggleSelectAll()
        tryCompare(checkbox, "checked", true)
        compare(page.selectedCount, 2)
    }

    function test_revoke_failure_keeps_dialog_open() {
        push([share("a")])
        page.selectIds(["a"])
        page.openRevoke(["a"])
        page.submitRevoke()
        backend.finishRevoke(false, "撤销失败")
        compare(revokeDialog.visible, true)
        compare(revokeDialog.errorText, "撤销失败")
    }
}
