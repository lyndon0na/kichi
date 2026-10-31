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
        property string contentView: "list"
        property int refreshCalls: 0
        property string refreshKind: ""
        property string openedId: ""
        property string playedId: ""
        property string revealedId: ""
        property string downloadedId: ""
        property int starCalls: 0
        property string starredId: ""
        property bool starredValue: false

        signal changed()
        signal fileActionDone(string action, bool ok, string message)
        signal thumbnailReady(string id, string path)
        signal shareActionDone(string action, bool ok, string message)

        function refreshLibrary(kind) {
            refreshCalls += 1
            refreshKind = kind
        }

        function setContentView(view) {
            contentView = view === "grid" ? "grid" : "list"
            changed()
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

        function setLibraryStarred(id, starred) {
            starCalls += 1
            starredId = id
            starredValue = starred
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
        backend.starCalls = 0
        backend.starredId = ""
        backend.starredValue = false
        backend.contentView = "list"

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

    function row(id, playable, kind, starred) {
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
            playable: playable === true,
            starred: starred === true
        }
    }

    function groupedRow(id, group) {
        const item = row(id, true)
        item.dateGroup = group
        return item
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

    function test_grid_mode_uses_shared_cards_and_keeps_direct_actions() {
        publish([row("folder", false, "folder"),
                 row("movie", true),
                 row("document", false)], true)
        var list = findChild(page, "libraryList")
        var grid = findChild(page, "libraryGrid")
        verify(list !== null && grid !== null)

        page.setContentView("grid")
        tryCompare(backend, "contentView", "grid")
        compare(list.visible, false)
        compare(grid.visible, true)
        compare(grid.count, 3)

        var movieCard = grid.itemAtIndex(1)
        verify(movieCard !== null)
        compare(movieCard.downloadNonPlayable, false)
        var detail = findChild(movieCard, "gridDetail")
        verify(detail !== null)
        compare(detail.text, "1.0 GB · 26-10-30 12:00")

        mouseDoubleClickSequence(movieCard, 30, 20)
        tryCompare(backend, "playedId", "movie")

        var folderCard = grid.itemAtIndex(0)
        verify(folderCard !== null)
        mouseDoubleClickSequence(folderCard, 30, 20)
        tryCompare(backend, "openedId", "folder")

        var documentCard = grid.itemAtIndex(2)
        verify(documentCard !== null)
        mouseDoubleClickSequence(documentCard, 30, 20)
        wait(20)
        compare(backend.openedId, "folder", "普通文件双击不能跳到所在位置")
        compare(backend.playedId, "movie", "普通文件双击不能播放")
    }

    function test_list_and_grid_keep_independent_scroll_positions() {
        const rows = []
        for (let i = 0; i < 40; ++i)
            rows.push(row("item-" + i, false))
        publish(rows, true)

        var list = findChild(page, "libraryList")
        verify(list !== null)
        list.contentY = 96
        wait(30)

        page.setContentView("grid")
        var grid = findChild(page, "libraryGrid")
        verify(grid !== null)
        tryCompare(grid, "visible", true)
        grid.contentY = 80
        wait(30)

        page.setContentView("list")
        tryCompare(backend, "contentView", "list")
        tryCompare(list, "contentY", 96)
    }

    function test_header_shows_count_and_refreshes_current_page() {
        publish([row("one", false), row("two", false)], true)
        compare(findChild(header, "libraryHeaderStatus").text, "共 2 项")

        var toggle = findChild(header, "viewToggleButton")
        var refresh = findChild(header, "libraryRefreshButton")
        verify(toggle !== null && refresh !== null)
        verify(toggle.x < refresh.x, "视图切换应排在刷新之前")

        mouseClick(findChild(header, "libraryRefreshButton"))
        tryCompare(backend, "refreshCalls", 1)
        compare(backend.refreshKind, "recent-saved")
    }

    function test_header_refresh_feedback_keeps_cached_count() {
        publish([row("one", false), row("two", false)], true)
        var status = findChild(header, "libraryHeaderStatus")
        var refresh = findChild(header, "libraryRefreshButton")
        verify(status !== null && refresh !== null)
        compare(status.text, "共 2 项")
        compare(refresh.text, "刷新")
        compare(refresh.enabled, true)

        // TTL 自动校准不进入可见 loading，计数和按钮完全不动。
        backend.libraryRevision += 1
        backend.changed()
        wait(20)
        compare(status.text, "共 2 项")
        compare(refresh.text, "刷新")
        compare(refresh.enabled, true)

        backend.libraryLoading = true
        backend.changed()
        wait(20)
        compare(status.text, "正在刷新…")
        compare(refresh.text, "刷新中…")
        compare(refresh.enabled, false)

        backend.libraryLoading = false
        backend.changed()
        wait(20)
        compare(status.text, "共 2 项")
        compare(refresh.text, "刷新")
        compare(refresh.enabled, true)
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

    function test_star_menu_updates_the_current_library_entry() {
        publish([row("plain", false)], true)
        var list = findChild(page, "libraryList")
        var item = list.itemAtIndex(0)
        verify(item !== null)

        mouseClick(item, 30, 20, Qt.RightButton)
        tryCompare(page.contextMenu, "visible", true)
        var starItem = findChild(page, "libraryStarItem")
        verify(starItem !== null)
        compare(starItem.text, "加星标")
        mouseClick(starItem)
        tryCompare(backend, "starCalls", 1)
        compare(backend.starredId, "plain")
        compare(backend.starredValue, true)
    }

    function test_starred_menu_offers_unstar() {
        publish([row("favorite", false, "file", true)], true)
        var list = findChild(page, "libraryList")
        var item = list.itemAtIndex(0)
        verify(item !== null)
        var star = findChild(item, "libraryStar")
        var slot = findChild(item, "libraryStarSlot")
        verify(star !== null && slot !== null)
        compare(star.visible, true)
        compare(star.filled, true)
        verify(slot.width > 0, "星标槽必须始终占位")

        mouseClick(item, 30, 20, Qt.RightButton)
        tryCompare(page.contextMenu, "visible", true)
        var starItem = findChild(page, "libraryStarItem")
        compare(starItem.text, "取消星标")
        mouseClick(starItem)
        tryCompare(backend, "starCalls", 1)
        compare(backend.starredId, "favorite")
        compare(backend.starredValue, false)
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

    function test_playback_history_list_has_fixed_date_sections() {
        backend.libraryKind = "playback-history"
        page.kind = "playback-history"
        page.dateSectionsEnabled = true
        publish([
            groupedRow("today", "today"),
            groupedRow("yesterday", "yesterday"),
            groupedRow("older", "earlier"),
            groupedRow("missing", "earlier")
        ], true)

        var list = findChild(page, "libraryList")
        verify(list !== null)
        compare(list.section.property, "dateGroup")
        compare(list.count, 4)

        var todayHeader = findChild(list, "libraryDateSection_today")
        var yesterdayHeader = findChild(list, "libraryDateSection_yesterday")
        var earlierHeader = findChild(list, "libraryDateSection_earlier")
        verify(todayHeader !== null)
        verify(yesterdayHeader !== null)
        verify(earlierHeader !== null)
        compare(todayHeader.title, "今天")
        compare(yesterdayHeader.title, "昨天")
        compare(earlierHeader.title, "更早")

        page.selectIndex(2)
        compare(list.currentIndex, 2)
        compare(page.currentEntryId, "older")
    }

    function test_playback_history_grid_keeps_sections_and_card_actions() {
        backend.libraryKind = "playback-history"
        page.kind = "playback-history"
        page.dateSectionsEnabled = true
        publish([
            groupedRow("today", "today"),
            groupedRow("yesterday", "yesterday"),
            groupedRow("older", "earlier")
        ], true)

        page.setContentView("grid")
        tryCompare(backend, "contentView", "grid")
        var grouped = findChild(page, "libraryGroupedGrid")
        verify(grouped !== null)
        tryCompare(grouped, "visible", true)
        tryCompare(grouped, "count", 3)

        var todayHeader = findChild(grouped, "libraryDateSection_today")
        var yesterdayHeader = findChild(grouped, "libraryDateSection_yesterday")
        var earlierHeader = findChild(grouped, "libraryDateSection_earlier")
        verify(todayHeader !== null)
        verify(yesterdayHeader !== null)
        verify(earlierHeader !== null)
        compare(todayHeader.title, "今天")
        compare(yesterdayHeader.title, "昨天")
        compare(earlierHeader.title, "更早")

        tryVerify(function () { return grouped.itemAtIndex(0) !== null })
        var card = grouped.itemAtIndex(0)
        compare(card.entry.id, "today")
        page.selectIndex(1)
        mouseClick(todayHeader, 20, 10)
        compare(page.currentEntryId, "yesterday", "分区头不能改变当前条目")

        mouseClick(card, 30, 20, Qt.RightButton)
        tryCompare(page.contextMenu, "visible", true)
        compare(page.blankContextMenu.visible, false)
        page.contextMenu.close()

        mouseDoubleClickSequence(card, 30, 20)
        tryCompare(backend, "playedId", "today")
    }

    function test_playback_history_hides_empty_date_sections() {
        backend.libraryKind = "playback-history"
        page.kind = "playback-history"
        page.dateSectionsEnabled = true
        publish([
            groupedRow("today", "today"),
            groupedRow("older", "earlier")
        ], true)

        page.setContentView("grid")
        var grouped = findChild(page, "libraryGroupedGrid")
        verify(grouped !== null)
        tryCompare(grouped, "visible", true)
        tryVerify(function () { return grouped.rows.length === 4 })
        verify(findChild(grouped, "libraryDateSection_today") !== null)
        verify(findChild(grouped, "libraryDateSection_yesterday") === null)
        verify(findChild(grouped, "libraryDateSection_earlier") !== null)
    }

    function test_playback_history_grouped_grid_keeps_its_scroll_position() {
        backend.libraryKind = "playback-history"
        page.kind = "playback-history"
        page.dateSectionsEnabled = true
        const rows = []
        for (let i = 0; i < 12; ++i)
            rows.push(groupedRow("today-" + i, "today"))
        for (let i = 0; i < 12; ++i)
            rows.push(groupedRow("yesterday-" + i, "yesterday"))
        for (let i = 0; i < 12; ++i)
            rows.push(groupedRow("earlier-" + i, "earlier"))
        publish(rows, true)

        var list = findChild(page, "libraryList")
        page.setContentView("grid")
        var grouped = findChild(page, "libraryGroupedGrid")
        var groupedList = findChild(page, "libraryGroupedList")
        verify(grouped !== null)
        verify(groupedList !== null)
        tryCompare(grouped, "visible", true)
        tryVerify(function () { return grouped.contentHeight > grouped.height })
        tryVerify(function () { return !page.pendingScrollRestore })
        groupedList.contentY = 240
        tryCompare(groupedList, "contentY", 240)

        page.setContentView("list")
        tryCompare(backend, "contentView", "list")
        tryVerify(function () { return !page.pendingScrollRestore })
        list.contentY = 96
        tryCompare(list, "contentY", 96)

        page.setContentView("grid")
        tryCompare(backend, "contentView", "grid")
        tryCompare(grouped, "contentY", 240)
    }
}
