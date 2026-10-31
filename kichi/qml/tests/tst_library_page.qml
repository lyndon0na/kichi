import QtQuick
import QtTest
import ".."

// 资料库三页共用的表格骨架。
// 三个数据源分支只替换后端的 libraryRows，页面本身必须能正确刷新、空态、
// 选中、直接打开与播放，否则单独实现某一条查询时很难定位问题。
TestCase {
    name: "LibraryPage"
    when: windowShown
    width: 760
    height: 520

    QtObject {
        id: backend

        property string libraryKind: "recent-saved"
        property var libraryRows: []
        property bool libraryLoading: false
        property bool libraryLoaded: false
        property string libraryError: ""
        property int libraryRevision: 0
        property bool playLoading: false
        property string playLoadingId: ""
        property var playingIds: []
        property var playMenu: []
        property bool filesLoading: false
        property bool fileActionRunning: false
        property bool shareActionRunning: false
        property string shareCreatedUrl: ""
        property string shareCreatedPassCode: ""
        property string shareCreatedExpiration: ""
        property int refreshCalls: 0
        property string refreshKind: ""
        property string openedId: ""
        property string playedId: ""
        property string revealedId: ""
        property string downloadedId: ""

        signal changed()
        signal thumbnailReady(string id, string path)
        signal shareActionDone(string action, bool ok, string message)

        function refreshLibrary(kind) {
            refreshCalls += 1
            refreshKind = kind
        }

        function openLibraryEntry(id) {
            openedId = id
        }

        function requestPlay(id) {
            playedId = id
        }

        function revealLibraryEntry(id) {
            revealedId = id
        }

        function downloadEntry(id) {
            downloadedId = id
        }

        function isDownloadPending(id) {
            return false
        }

        function hasPartialDownload(id) {
            return false
        }

        function cancelPlayLoading() {}
        function cancelPlay() {}
        function playSelected(index) {}
        function thumbnailPath(id) {
            return ""
        }
        function requestThumbnail(id) {}
    }

    Component {
        id: pageComponent

        LibraryPage {
            width: 760
            height: 520
            kind: "recent-saved"
            primaryHeader: "大小"
            secondaryHeader: "保存时间"
            emptyText: "最近保存为空"
        }
    }

    Component {
        id: headerComponent

        LibraryHeaderControls {}
    }

    property var page: null
    property var header: null

    function init() {
        backend.libraryKind = "recent-saved"
        backend.libraryRows = []
        backend.libraryLoading = false
        backend.libraryLoaded = false
        backend.libraryError = ""
        backend.libraryRevision = 0
        backend.refreshCalls = 0
        backend.refreshKind = ""
        backend.openedId = ""
        backend.playedId = ""
        backend.revealedId = ""
        backend.downloadedId = ""

        page = pageComponent.createObject(parent, {
            visible: true,
            active: true,
            backend: backend
        })
        verify(page !== null, "LibraryPage 创建失败")
        header = headerComponent.createObject(parent, {
            width: 700,
            target: page
        })
        verify(header !== null, "LibraryHeaderControls 创建失败")
        wait(20)
        backend.refreshCalls = 0
    }

    function cleanup() {
        if (header)
            header.destroy()
        if (page)
            page.destroy()
        header = null
        page = null
    }

    function row(id, playable, kind) {
        return {
            id: id,
            name: id + ".mkv",
            kind: kind || "file",
            sizeText: "1.0 GB",
            modifiedText: "2026-10-30 12:00",
            createdText: "2026-10-29 12:00",
            primaryText: "1.0 GB",
            secondaryText: "2026-10-30 12:00",
            hasThumbnail: false,
            video: playable === true,
            playable: playable === true
        }
    }

    function publish(rows, loaded) {
        backend.libraryRows = rows
        backend.libraryLoaded = loaded !== false
        backend.libraryRevision += 1
        backend.changed()
        wait(20)
    }

    function test_entering_page_refreshes_and_empty_state_is_shown() {
        page.active = false
        wait(10)
        backend.refreshCalls = 0
        page.active = true
        tryCompare(backend, "refreshCalls", 1)
        compare(backend.refreshKind, "recent-saved")

        publish([], true)
        var empty = findChild(page, "libraryEmptyHint")
        verify(empty !== null)
        compare(empty.visible, true)
    }

    function test_only_active_library_page_owns_quality_menu() {
        var playMenu = findChild(page, "libraryPlayMenu")
        verify(playMenu !== null)
        compare(playMenu.backend, backend)

        page.active = false
        tryVerify(function () { return playMenu.backend === null })

        page.active = true
        tryVerify(function () { return playMenu.backend === backend })
    }

    function test_rows_use_backend_order_and_selection() {
        publish([row("new", false, "folder"), row("old", false, "folder")], true)
        var list = findChild(page, "libraryList")
        verify(list !== null)
        compare(list.count, 2)
        compare(list.itemAtIndex(0).entry.id, "new")
        compare(list.itemAtIndex(1).entry.id, "old")

        list.currentIndex = 1
        page.activateCurrent()
        compare(backend.openedId, "old")
    }

    function test_playable_row_requests_play() {
        publish([row("movie", true)], true)
        var list = findChild(page, "libraryList")
        verify(list !== null)
        list.currentIndex = 0
        page.activateCurrent()
        compare(backend.playedId, "movie")
    }

    function test_double_click_playable_row_requests_play_without_runtime_error() {
        publish([row("movie", true)], true)
        var list = findChild(page, "libraryList")
        verify(list !== null)
        var item = list.itemAtIndex(0)
        verify(item !== null)
        mouseDoubleClickSequence(item, 30, 20)
        tryCompare(backend, "playedId", "movie")
    }

    function test_row_play_signal_requests_play_from_delegate() {
        publish([row("movie", true)], true)
        var list = findChild(page, "libraryList")
        verify(list !== null)
        var item = list.itemAtIndex(0)
        verify(item !== null)
        item.playRequested("movie", Qt.point(12, 18))
        tryCompare(backend, "playedId", "movie")
    }

    function test_plain_file_double_click_does_not_open_location() {
        publish([row("document", false)], true)
        var list = findChild(page, "libraryList")
        verify(list !== null)
        var item = list.itemAtIndex(0)
        verify(item !== null)
        mouseDoubleClickSequence(item, 30, 20)
        wait(20)
        compare(backend.openedId, "")
        compare(backend.playedId, "")
    }

    function test_double_click_folder_opens_folder_itself() {
        publish([row("folder", false, "folder")], true)
        var list = findChild(page, "libraryList")
        verify(list !== null)
        var item = list.itemAtIndex(0)
        verify(item !== null)
        mouseDoubleClickSequence(item, 30, 20)
        tryCompare(backend, "openedId", "folder")
    }

    function test_loading_and_error_suppress_empty_hint() {
        backend.libraryLoading = true
        backend.libraryRevision += 1
        backend.changed()
        wait(10)
        compare(findChild(page, "libraryEmptyHint").visible, false)

        backend.libraryLoading = false
        backend.libraryError = "网络失败"
        backend.libraryRevision += 1
        backend.changed()
        wait(10)
        compare(findChild(page, "libraryEmptyHint").visible, false)
        compare(findChild(page, "libraryError").visible, true)
        compare(findChild(page, "libraryFailureHint").visible, true)
    }

    function test_refresh_failure_keeps_existing_rows_and_hides_failure_hint() {
        publish([row("one", false), row("two", false)], true)
        var list = findChild(page, "libraryList")
        verify(list !== null)
        compare(list.count, 2)

        backend.libraryLoading = false
        backend.libraryLoaded = false
        backend.libraryError = "刷新失败"
        backend.libraryRevision += 1
        backend.changed()
        wait(20)

        compare(list.count, 2)
        compare(findChild(page, "libraryError").visible, true)
        compare(findChild(page, "libraryEmptyHint").visible, false)
        compare(findChild(page, "libraryFailureHint").visible, false)
    }

    function test_refresh_keeps_current_row_when_count_changes() {
        publish([row("one", false), row("two", false), row("three", false)], true)
        var list = findChild(page, "libraryList")
        verify(list !== null)
        page.selectIndex(1)

        publish([row("one", false), row("two", false),
                 row("three", false), row("four", false)], true)
        compare(list.currentIndex, 1)
    }

    function test_refresh_keeps_selected_entry_when_order_changes() {
        publish([row("one", false), row("two", false), row("three", false)], true)
        var list = findChild(page, "libraryList")
        verify(list !== null)
        page.selectIndex(2)

        publish([row("three", false), row("two", false), row("one", false)], true)
        compare(list.currentIndex, 0)
        compare(list.itemAtIndex(0).entry.id, "three")
    }

    function test_header_shows_count_and_refreshes_current_page() {
        publish([row("one", false), row("two", false)], true)
        compare(findChild(header, "libraryHeaderStatus").text, "共 2 项")

        mouseClick(findChild(header, "libraryRefreshButton"))
        tryCompare(backend, "refreshCalls", 1)
        compare(backend.refreshKind, "recent-saved")
    }

    function test_right_click_opens_common_menu_and_reveals_location() {
        publish([row("movie", true)], true)
        var list = findChild(page, "libraryList")
        var item = list.itemAtIndex(0)
        verify(item !== null)

        mouseClick(item, 30, 20, Qt.RightButton)
        tryCompare(page.contextMenu, "visible", true)
        var playItem = findChild(page, "libraryPlayItem")
        var revealItem = findChild(page, "libraryRevealItem")
        verify(playItem !== null)
        verify(revealItem !== null)
        compare(playItem.visible, true)
        compare(revealItem.text, "打开文件所在位置")

        mouseClick(revealItem)
        tryCompare(backend, "revealedId", "movie")
        page.contextMenu.close()
    }

    function test_folder_menu_opens_folder_and_downloads() {
        publish([row("folder", false, "folder")], true)
        var list = findChild(page, "libraryList")
        var item = list.itemAtIndex(0)
        verify(item !== null)

        mouseClick(item, 30, 20, Qt.RightButton)
        tryCompare(page.contextMenu, "visible", true)
        var playItem = findChild(page, "libraryPlayItem")
        var openItem = findChild(page, "libraryOpenFolderItem")
        var revealItem = findChild(page, "libraryRevealItem")
        var downloadItem = findChild(page, "libraryDownloadItem")
        compare(playItem.visible, false)
        compare(openItem.visible, true)
        compare(revealItem.text, "打开目录所在位置")
        compare(downloadItem.text, "下载整个文件夹到本地")

        mouseClick(openItem)
        tryCompare(backend, "openedId", "folder")
        page.contextMenu.close()

        mouseClick(item, 30, 20, Qt.RightButton)
        tryCompare(page.contextMenu, "visible", true)
        mouseClick(findChild(page, "libraryDownloadItem"))
        tryCompare(backend, "downloadedId", "folder")
        page.contextMenu.close()
    }

    function test_plain_file_menu_hides_play_and_open_folder() {
        publish([row("document", false)], true)
        var list = findChild(page, "libraryList")
        var item = list.itemAtIndex(0)
        verify(item !== null)

        mouseClick(item, 30, 20, Qt.RightButton)
        tryCompare(page.contextMenu, "visible", true)
        compare(findChild(page, "libraryPlayItem").visible, false)
        compare(findChild(page, "libraryOpenFolderItem").visible, false)
        compare(findChild(page, "libraryRevealItem").text, "打开文件所在位置")
        page.contextMenu.close()
    }

    function test_blank_right_click_refreshes() {
        publish([row("one", false)], true)
        var list = findChild(page, "libraryList")
        verify(list !== null)
        mouseClick(list, 80, list.height - 10, Qt.RightButton)
        tryCompare(page.blankContextMenu, "visible", true)
        var refreshItem = findChild(page, "libraryRefreshItem")
        verify(refreshItem !== null)
        mouseClick(refreshItem)
        tryCompare(backend, "refreshCalls", 1)
        page.blankContextMenu.close()
    }
}
