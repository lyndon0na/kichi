import QtQuick
import QtTest
import ".."

TestCase {
    name: "NewTaskMagnet"
    when: windowShown
    width: 900
    height: 700

    QtObject {
        id: backend

        property var magnetFiles: []
        property string magnetRootName: "示例磁链"
        property string magnetSummary: "共 1 个文件 · 1 GB；全部可秒传"
        property string magnetFilesError: ""
        property bool magnetFilesLoading: false
        property bool magnetSaving: false
        property string magnetSaveStatus: ""

        property string lastLink: ""
        property string lastSubmitted: ""
        property string lastSubmittedFolder: ""

        signal magnetPreviewFinished(bool ok, string message, bool canFallback)
        signal magnetSaveFinished(int saved, int unavailable, int failed, string message)

        function currentPath() {
            return [{ id: "", name: "全部文件" }]
        }

        function previewMagnetFiles(link) {
            lastLink = link
            magnetFilesLoading = true
        }

        function finishPreview(ok, message, canFallback) {
            magnetFilesLoading = false
            magnetPreviewFinished(ok, message, canFallback)
        }

        function addTask(text) {
            lastSubmitted = text
            lastSubmittedFolder = ""
        }

        function addTaskToFolder(text, folderId) {
            lastSubmitted = text
            lastSubmittedFolder = folderId
        }

        function saveMagnetFiles(indices, folderId, keepStructure) {
        }
    }

    NewTaskDialog {
        id: dialog
        backend: backend
    }

    property var input: null
    property var footerOk: null
    property var footerCancel: null
    property var magnetDialog: null

    function init() {
        backend.magnetFiles = [
            { index: 0, name: "第 1 集.mkv", path: "", folders: [], size: "1 GB",
              note: "", available: true }
        ]
        backend.magnetRootName = "示例磁链"
        backend.magnetSummary = "共 1 个文件 · 1 GB；全部可秒传"
        backend.magnetFilesError = ""
        backend.magnetFilesLoading = false
        backend.magnetSaving = false
        backend.magnetSaveStatus = ""
        backend.lastLink = ""
        backend.lastSubmitted = ""
        backend.lastSubmittedFolder = ""
        if (magnetDialog && magnetDialog.visible)
            magnetDialog.close()
        if (dialog.visible)
            dialog.close()
    }

    function cleanup() {
        if (magnetDialog && magnetDialog.visible)
            magnetDialog.close()
        if (dialog.visible)
            dialog.close()
    }

    function open() {
        dialog.open()
        tryCompare(dialog, "visible", true)
        input = findChild(dialog, "newTaskInput")
        // 用本弹窗专属的按钮名：内嵌目录选择器 / 磁链弹窗也有同名页脚。
        footerOk = findChild(dialog, "newTaskDialogOk")
        footerCancel = findChild(dialog, "newTaskDialogCancel")
        magnetDialog = findChild(dialog, "magnetFilesDialog")
    }

    function test_single_magnet_is_parsed_before_submitting() {
        open()
        input.text = "magnet:?xt=urn:btih:abc&dn=show"
        mouseClick(footerOk)

        compare(backend.lastLink, "magnet:?xt=urn:btih:abc&dn=show")
        compare(backend.lastSubmitted, "", "解析期间不能直接提交云下载")
        compare(dialog.parsingMagnet, true)
        compare(footerOk.text, "解析中…")
        compare(footerOk.enabled, false)
        compare(footerCancel.enabled, false)
        compare(dialog.torrentNotice, "正在解析磁链……")

        backend.finishPreview(true, "", false)
        tryCompare(magnetDialog, "visible", true)
        compare(dialog.parsingMagnet, false)
        compare(magnetDialog.targetFolderId, "", "My Pack 没有目录 id，秒传落到根目录")
        compare(magnetDialog.targetFolderPath, "全部文件")
        compare(findChild(magnetDialog, "magnetFilesList").count, 1)
        compare(backend.lastSubmitted, "")
    }

    function test_cloud_download_falls_back_when_nothing_cached() {
        backend.magnetFiles = [
            { index: 0, name: "第 1 集.mkv", path: "", folders: [], size: "1 GB",
              note: "云端暂无缓存，无法秒传", available: false }
        ]
        open()
        input.text = "magnet:?xt=urn:btih:abc"
        mouseClick(footerOk)
        backend.finishPreview(true, "", false)

        tryCompare(dialog, "visible", false)
        compare(backend.lastSubmitted, "magnet:?xt=urn:btih:abc")
        compare(magnetDialog.visible, false)
    }

    function test_cloud_download_falls_back_when_parsing_fails() {
        open()
        input.text = "magnet:?xt=urn:btih:abc"
        mouseClick(footerOk)
        backend.finishPreview(false, "磁力链接无效或没有可用文件", true)

        tryCompare(dialog, "visible", false)
        compare(backend.lastSubmitted, "magnet:?xt=urn:btih:abc")
        compare(magnetDialog.visible, false)
    }

    function test_session_failure_stays_open_without_fallback() {
        open()
        input.text = "magnet:?xt=urn:btih:abc"
        mouseClick(footerOk)
        backend.finishPreview(false, "登录状态失效", false)

        compare(dialog.visible, true)
        compare(dialog.parsingMagnet, false)
        compare(backend.lastSubmitted, "", "登录态失败不应再提交云下载")
        compare(dialog.torrentNoticeError, true)
        compare(dialog.torrentNotice, "登录状态失效")
        compare(footerOk.text, "添加")
        compare(footerOk.enabled, true)
    }

    function test_multiple_links_submit_without_parsing() {
        open()
        input.text = "magnet:?xt=urn:btih:abc\nmagnet:?xt=urn:btih:def"
        mouseClick(footerOk)

        compare(backend.lastLink, "")
        compare(backend.lastSubmitted,
                "magnet:?xt=urn:btih:abc\nmagnet:?xt=urn:btih:def")
        tryCompare(dialog, "visible", false)
    }

    function test_non_magnet_link_submits_without_parsing() {
        open()
        input.text = "https://example.com/a.mkv"
        mouseClick(footerOk)

        compare(backend.lastLink, "")
        compare(backend.lastSubmitted, "https://example.com/a.mkv")
        tryCompare(dialog, "visible", false)
    }

    function test_bare_hex_infohash_is_parsed_before_submitting() {
        open()
        const hash = "0123456789ABCDEF0123456789ABCDEF01234567"
        input.text = hash

        compare(dialog.canSubmit, true)
        compare(dialog.magnetLink, hash)
        mouseClick(footerOk)

        compare(backend.lastLink, hash, "纯 hash 直接交给后端规范化")
        compare(dialog.parsingMagnet, true)
        compare(backend.lastSubmitted, "")

        backend.finishPreview(true, "", false)
        tryCompare(magnetDialog, "visible", true)
        compare(backend.lastSubmitted, "")
    }

    function test_bare_base32_infohash_is_accepted() {
        open()
        const hash = "AERUKZ4JVPG66AJDIVTYTK6N54ASGRLH"
        input.text = hash

        compare(dialog.canSubmit, true)
        compare(dialog.magnetLink, hash)
        mouseClick(footerOk)
        compare(backend.lastLink, hash)
        compare(dialog.parsingMagnet, true)
    }

    function test_hash_with_wrong_length_is_rejected() {
        open()
        input.text = "0123456789abcdef"
        compare(dialog.canSubmit, false)
        compare(dialog.invalidLine, 1)
        compare(dialog.magnetLink, "")

        input.text = "0123456789ABCDEF0123456789ABCDEF0123456"
        compare(dialog.canSubmit, false, "39 位十六进制也不接受")
    }

    function test_base32_hash_with_invalid_character_is_rejected() {
        open()
        input.text = "AERUKZ4JVPG66AJDIVTYTK6N54ASGRL0"
        compare(dialog.canSubmit, false)
        compare(dialog.invalidLine, 1)
    }
}
