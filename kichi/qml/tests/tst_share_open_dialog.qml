import QtQuick
import QtTest
import "../shares"

TestCase {
    name: "ShareOpenDialog"
    when: windowShown
    width: 900
    height: 700

    QtObject {
        id: backend

        property bool shareOpenBusy: false
        property bool shareOpenReady: false
        property bool shareOpenNeedsPassCode: false
        property string shareOpenError: ""
        property string shareOpenTitle: ""
        property string shareOpenOwner: ""
        property string shareOpenMeta: ""
        property var shareOpenRows: []
        property var shareOpenPath: []
        property bool shareOpenTruncated: false
        property bool shareSaveRunning: false
        property var moveTargets: []
        property bool moveTargetsLoading: false
        property string moveTargetsError: ""
        property int moveTargetsRevision: 0

        property string lastLink: ""
        property string lastCode: ""
        property var lastRestoreIds: []
        property var lastParentId: null

        signal changed()
        signal shareRestoreDone(bool ok, string message)

        function openShareLink(text, passCode) {
            lastLink = text
            lastCode = passCode
            shareOpenBusy = true
        }

        function openShareFolder(folderId, folderName) {}

        function shareOpenGoUp() {}

        function closeShareOpen() {
            shareOpenBusy = false
            shareOpenReady = false
            shareOpenError = ""
        }

        function restoreShareFiles(ids, parentId) {
            lastRestoreIds = ids.slice()
            lastParentId = parentId
            shareSaveRunning = true
            return true
        }

        function loadMoveTargets(parentId, force) {
            moveTargetsRevision += 1
        }

        function cancelMoveTargets() {}

        function finishOpen() {
            shareOpenBusy = false
            shareOpenReady = true
            shareOpenTitle = "剧集合集"
            shareOpenOwner = "来自 someone"
            shareOpenMeta = "2 个条目 · 永久有效"
            shareOpenPath = [{ id: "", name: "全部文件" }]
            shareOpenRows = [
                { id: "f1", name: "a.mkv", kind: "file", isFolder: false, sizeText: "1 MB" },
                { id: "d1", name: "第二季", kind: "folder", isFolder: true, sizeText: "" }
            ]
            changed()
        }

        function finishRestore(ok, message) {
            shareSaveRunning = false
            shareRestoreDone(ok, message)
        }
    }

    ShareOpenDialog {
        id: dialog
        backend: backend
    }

    property var openDialog: findChild(dialog, "shareOpenDialog")
    property var saveDialog: findChild(dialog, "shareSaveDialog")
    property var linkInput: findChild(dialog, "shareOpenLinkInput")
    property var codeInput: findChild(dialog, "shareOpenPassCodeInput")
    property var titleLabel: findChild(dialog, "shareOpenTitle")
    property var doneLabel: findChild(dialog, "shareOpenDoneMessage")

    function init() {
        backend.shareOpenBusy = false
        backend.shareOpenReady = false
        backend.shareOpenNeedsPassCode = false
        backend.shareOpenError = ""
        backend.shareOpenRows = []
        backend.shareOpenPath = []
        backend.shareSaveRunning = false
        backend.lastLink = ""
        backend.lastCode = ""
        backend.lastRestoreIds = []
        backend.lastParentId = null
        if (openDialog.visible)
            openDialog.close()
    }

    function cleanup() {
        if (saveDialog.visible)
            saveDialog.close()
        if (openDialog.visible)
            openDialog.close()
    }

    function openInput() {
        dialog.openDialog()
        tryCompare(openDialog, "visible", true)
    }

    function enterBrowse() {
        openInput()
        backend.finishOpen()
        tryCompare(openDialog, "stage", "browse")
    }

    function test_submit_needs_a_link_and_passes_code() {
        openInput()
        // 弹窗内容（含页脚）在第一次打开时才创建，必须打开后再查找。
        const footerOk = findChild(dialog, "dialogFooterOk")
        verify(footerOk !== null)
        compare(footerOk.enabled, false, "没有链接时不能提交")

        linkInput.text = "看看这个 https://mypikpak.com/s/ABC123 访问密码：p9k2"
        compare(footerOk.enabled, true)
        codeInput.text = "typed1"
        dialog.submit()

        compare(backend.lastLink, linkInput.text)
        // 显式填写的密码优先于文本里解析出的
        compare(backend.lastCode, "typed1")
    }

    function test_ready_switches_to_browse_and_lists_entries() {
        enterBrowse()
        compare(titleLabel.text, "剧集合集")

        const list = findChild(dialog, "shareOpenList")
        verify(list !== null)
        compare(list.count, 2)
    }

    function test_needs_pass_code_shows_hint() {
        openInput()
        backend.shareOpenNeedsPassCode = true
        backend.shareOpenError = "这个分享需要访问密码"
        backend.changed()
        const hint = findChild(dialog, "shareOpenNeedsPassCode")
        verify(hint !== null)
        compare(hint.visible, true)
    }

    function test_selection_is_restored_and_success_shows_done() {
        enterBrowse()
        dialog.toggle("f1")
        dialog.toggle("d1")
        compare(dialog.selectedCount, 2)

        dialog.openSave()
        tryCompare(saveDialog, "visible", true)
        saveDialog.apply()
        compare(backend.lastRestoreIds.length, 2)
        compare(backend.lastParentId, "")

        backend.finishRestore(true, "已转存到所选文件夹")
        tryCompare(openDialog, "stage", "done")
        compare(doneLabel.text, "已转存到所选文件夹")
    }

    function test_failure_keeps_save_dialog_open_with_reason() {
        enterBrowse()
        dialog.toggle("f1")
        dialog.openSave()
        tryCompare(saveDialog, "visible", true)
        saveDialog.apply()

        backend.finishRestore(false, "存储空间不足")
        compare(openDialog.stage, "browse")
        compare(saveDialog.visible, true)
        compare(saveDialog.errorText, "存储空间不足")
    }
}
