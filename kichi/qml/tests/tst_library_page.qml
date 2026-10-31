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
        property int refreshCalls: 0
        property string refreshKind: ""
        property string openedId: ""
        property string playedId: ""

        signal changed()
        signal thumbnailReady(string id, string path)

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

    property var page: null

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

        page = pageComponent.createObject(parent, {
            visible: true,
            backend: backend
        })
        verify(page !== null, "LibraryPage 创建失败")
        wait(20)
        backend.refreshCalls = 0
    }

    function cleanup() {
        if (page)
            page.destroy()
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

    function test_visible_refreshes_and_empty_state_is_shown() {
        page.visible = false
        wait(10)
        backend.refreshCalls = 0
        page.visible = true
        tryCompare(backend, "refreshCalls", 1)
        compare(backend.refreshKind, "recent-saved")

        publish([], true)
        var empty = findChild(page, "libraryEmptyHint")
        verify(empty !== null)
        compare(empty.visible, true)
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
    }
}
