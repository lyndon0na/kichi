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
        property string libraryStatus: ""
        property int libraryRevision: 0
        property int libraryDataRevision: 0
        property bool playLoading: false
        property string playLoadingId: ""
        property var playingIds: []
        property var playMenu: []
        property bool filesLoading: false
        property bool fileActionRunning: false
        property var fileActionIds: []
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
        signal fileActionDone(string action, bool ok, string message, var ids)
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
        backend.libraryStatus = ""
        backend.libraryRevision = 0
        backend.libraryDataRevision = 0
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
        backend.fileActionIds = []

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
        backend.libraryDataRevision += 1
        backend.changed()
        wait(20)
    }

    function test_cached_rows_survive_switching_library_collections() {
        const recentRows = [row("recent-a", false), row("recent-b", false)]
        publish(recentRows, true)
        const list = findChild(page, "libraryList")
        verify(list !== null)
        compare(list.count, 2)

        const recentRevision = backend.libraryDataRevision
        page.active = false
        backend.libraryKind = "starred"
        backend.libraryRows = [row("starred-a", false, "file", true)]
        backend.libraryRevision += 1
        backend.libraryDataRevision += 1
        backend.changed()
        wait(20)
        compare(list.count, 0, "离开前台后释放行模型")

        backend.libraryKind = "recent-saved"
        backend.libraryRows = recentRows
        backend.libraryRevision += 1
        backend.libraryDataRevision = recentRevision
        backend.changed()
        page.active = true
        wait(20)
        compare(list.count, 2, "切回未变化集合时保留原模型")

        const changedRows = [row("recent-c", false), row("recent-d", false)]
        backend.libraryRows = changedRows
        backend.libraryRevision += 1
        backend.libraryDataRevision += 1
        backend.changed()
        tryCompare(list, "count", 2)
        compare(list.itemAtIndex(0).entry.id, "recent-c")
        compare(list.itemAtIndex(1).entry.id, "recent-d")
    }

    function test_inactive_page_releases_grouped_models_and_restores_them() {
        backend.libraryKind = "playback-history"
        page.kind = "playback-history"
        page.dateSectionsEnabled = true
        publish(playbackRowsPerGroup(), true)
        page.setContentView("grid")

        var grouped = findChild(page, "libraryGroupedGrid")
        verify(grouped !== null)
        tryCompare(grouped, "visible", true)
        tryVerify(function () { return grouped.rows.length > 0 })

        page.active = false
        tryVerify(function () { return grouped.rows.length === 0 },
                  2000, "隐藏资料库页应释放分区网格行")

        page.active = true
        tryVerify(function () { return grouped.rows.length > 0 },
                  2000, "回到资料库页后应恢复分区网格行")
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

    function test_empty_loads_use_skeleton_each_time() {
        const freshPage = pageComponent.createObject(parent, {
            visible: false,
            active: false,
            backend: backend
        })
        verify(freshPage !== null)

        backend.libraryRows = []
        backend.libraryLoaded = false
        backend.libraryLoading = true
        backend.libraryRevision += 1
        backend.libraryDataRevision += 1
        backend.changed()
        freshPage.visible = true
        freshPage.active = true
        wait(30)

        const skeleton = findChild(freshPage, "pageLoadingSkeleton")
        const spinner = findChild(freshPage, "libraryLoadingIndicator")
        verify(skeleton !== null && spinner !== null)
        compare(skeleton.visible, true, "空内容读取显示骨架")
        compare(spinner.visible, false, "空内容读取不叠原来的转圈")

        backend.libraryLoading = false
        publish([row("first", false)], true)
        wait(30)
        compare(skeleton.visible, false, "数据到达后收起骨架")

        backend.libraryLoading = true
        backend.libraryRevision += 1
        backend.changed()
        wait(30)
        compare(skeleton.visible, false, "有旧内容时刷新保留内容，不闪骨架")

        backend.libraryRows = []
        backend.libraryLoaded = false
        backend.libraryLoading = true
        backend.libraryRevision += 1
        backend.libraryDataRevision += 1
        backend.changed()
        wait(30)
        compare(skeleton.visible, true, "再次空内容读取仍显示骨架")
        compare(spinner.visible, false, "再次空内容读取不叠原来的转圈")

        freshPage.destroy()
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

    function test_header_shows_actions_without_status_and_refreshes_current_page() {
        publish([row("one", false), row("two", false)], true)
        compare(findChild(header, "libraryHeaderStatus"), null,
                "资料库标题旁不再显示条目数或读取状态")

        var toggle = findChild(header, "viewToggleButton")
        var refresh = findChild(header, "libraryRefreshButton")
        verify(toggle !== null && refresh !== null)
        verify(toggle.x < refresh.x, "视图切换应排在刷新之前")

        mouseClick(findChild(header, "libraryRefreshButton"))
        tryCompare(backend, "refreshCalls", 1)
        compare(backend.refreshKind, "recent-saved")
    }

    function test_header_refresh_feedback_keeps_actions_only() {
        publish([row("one", false), row("two", false)], true)
        var refresh = findChild(header, "libraryRefreshButton")
        verify(refresh !== null)
        var spinner = findChild(refresh, "buttonBusyIndicator")
        verify(spinner !== null)
        compare(refresh.text, "刷新")
        compare(refresh.enabled, true)
        compare(spinner.running, false)
        var idleWidth = refresh.width

        // TTL 自动校准不进入可见 loading，按钮完全不动。
        backend.libraryRevision += 1
        backend.changed()
        wait(20)
        compare(refresh.text, "刷新")
        compare(refresh.enabled, true)
        compare(spinner.running, false)

        backend.libraryLoading = true
        backend.changed()
        wait(20)
        compare(refresh.text, "刷新")
        compare(refresh.enabled, false)
        compare(spinner.running, true)
        compare(refresh.width, idleWidth)

        backend.libraryLoading = false
        backend.changed()
        wait(20)
        compare(refresh.text, "刷新")
        compare(refresh.enabled, true)
        compare(spinner.running, false)
        compare(refresh.width, idleWidth)
    }

    function test_header_refresh_is_independent_from_file_actions() {
        publish([row("one", false), row("two", false)], true)
        var refresh = findChild(header, "libraryRefreshButton")
        verify(refresh !== null)
        compare(refresh.enabled, true)

        // 星标等文件写操作不是刷新；不能让刷新按钮跟着变淡 / 禁用。
        backend.fileActionIds = ["one"]
        backend.changed()
        wait(20)
        compare(refresh.text, "刷新")
        compare(refresh.enabled, true)

        backend.fileActionIds = []
        backend.changed()
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

    function test_unrelated_action_does_not_disable_library_star() {
        publish([row("plain", false)], true)
        var list = findChild(page, "libraryList")
        var item = list.itemAtIndex(0)
        verify(item !== null)

        backend.fileActionIds = ["other"]
        mouseClick(item, 30, 20, Qt.RightButton)
        tryCompare(page.contextMenu, "visible", true)
        var starItem = findChild(page, "libraryStarItem")
        verify(starItem !== null && starItem.enabled, "其它条目在写不应禁用当前条目星标")

        backend.fileActionIds = ["plain"]
        backend.changed()
        compare(starItem.enabled, false, "同一目标在写时只禁用这一项")
        page.contextMenu.close()
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

        var list = findChild(page, "libraryGroupedListView")
        verify(list !== null)
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
        compare(grouped.cellSlotWidth,
                Math.floor(grouped.availableWidth / grouped.columns))
        const occupiedWidth = (grouped.columns - 1) * grouped.cellSlotWidth
                              + grouped.itemWidth
        verify(occupiedWidth <= grouped.availableWidth)
        const visualRightMargin = grouped.rightMargin
                                  + grouped.availableWidth - occupiedWidth
        const expectedRightMargin = grouped.sideMargin
                                    + (grouped.scrollBarVisible
                                       ? grouped.scrollBarSlot : 0)
        compare(grouped.scrollBarVisible, grouped.contentHeight > grouped.height)
        verify(visualRightMargin >= expectedRightMargin)
        verify(visualRightMargin - expectedRightMargin < grouped.columns,
               "播放历史网格末列不能额外留下整段列间距")

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

    function test_playback_history_list_actions_and_navigation_skip_headers() {
        backend.libraryKind = "playback-history"
        page.kind = "playback-history"
        page.dateSectionsEnabled = true
        publish([groupedRow("today", "today"), groupedRow("yesterday", "yesterday"),
                 groupedRow("older", "earlier")], true)
        const list = findChild(page, "libraryGroupedListView")
        const view = findChild(page, "libraryDateList")
        const first = list.itemAtIndex(0)
        verify(first !== null)
        mouseClick(first, 30, 20)
        compare(list.currentIndex, 0)
        keyClick(Qt.Key_Down)
        compare(list.currentIndex, 1)
        compare(page.currentEntryId, "yesterday")
        keyClick(Qt.Key_End)
        compare(list.currentIndex, 2)
        compare(page.currentEntryId, "older")
        keyClick(Qt.Key_Home)
        compare(list.currentIndex, 0)
        keyClick(Qt.Key_Return)
        compare(backend.playedId, "today")

        mouseClick(first, 30, 20, Qt.RightButton)
        tryCompare(page.contextMenu, "visible", true)
        compare(page.blankContextMenu.visible, false)
        page.contextMenu.close()
        mouseDoubleClickSequence(first, 30, 20)
        compare(backend.playedId, "today")
        const header = findChild(view, "libraryDateSection_yesterday")
        mouseClick(header, 20, 10)
        compare(page.currentEntryId, "today", "日期分区头不能改变选择")
    }

    function test_playback_history_exact_layout_survives_resize_and_refresh_data() {
        return [{ tag: "list", grid: false }, { tag: "grid", grid: true }]
    }

    function test_playback_history_exact_layout_survives_resize_and_refresh(data) {
        backend.libraryKind = "playback-history"
        page.kind = "playback-history"
        page.dateSectionsEnabled = true
        const rows = []
        for (let i = 0; i < 66; ++i)
            rows.push(groupedRow("history-" + i,
                                 i < 2 ? "today" : i < 7 ? "yesterday" : "earlier"))
        publish(rows, true)
        if (data.grid)
            page.setContentView("grid")
        const container = page.activeView
        const view = findChild(page, data.grid ? "libraryGroupedList" : "libraryDateList")
        verify(view !== null)
        const total = view.contentHeight
        const ratio = view.visibleArea.heightRatio
        const bottom = total - view.height
        for (let y = 0; y < bottom + 100; y += 100) {
            view.contentY = Math.min(y, bottom)
            wait(1)
            compare(view.contentHeight, total)
            compare(view.originY, 0)
            compare(view.visibleArea.heightRatio, ratio)
        }
        verify(container.itemAtIndex(65) !== null)
        page.width = 1040
        wait(20)
        container.positionViewAtIndex(65, ListView.End)
        const last = container.itemAtIndex(65)
        verify(last !== null)
        verify(last.mapToItem(view, 0, last.height).y <= view.height)
        verify(last.mapToItem(view, 0, 0).y >= 0)

        // 刷新缩短到两组、再清空时，底部的旧委托必须安全回收。
        publish(rows.slice(0, 3), true)
        tryCompare(container, "contentY", 0)
        verify(container.itemAtIndex(0) !== null)
        publish([], true)
        tryCompare(container, "contentHeight", 0)
        compare(view.instantiatedRows, 0)
    }

    function test_playback_history_grid_drops_scrollbar_lane_when_content_fits() {
        backend.libraryKind = "playback-history"
        page.kind = "playback-history"
        page.dateSectionsEnabled = true
        publish([groupedRow("today", "today")], true)

        page.setContentView("grid")
        var grouped = findChild(page, "libraryGroupedGrid")
        verify(grouped !== null)
        tryCompare(grouped, "visible", true)
        compare(grouped.scrollBarVisible, false)
        verify(grouped.contentHeight <= grouped.height)

        const occupiedWidth = (grouped.columns - 1) * grouped.cellSlotWidth
                              + grouped.itemWidth
        const actualRightMargin = grouped.rightMargin
                                  + grouped.availableWidth - occupiedWidth
        compare(actualRightMargin, grouped.sideMargin,
                "无滚动条时左右页面边距必须一致")
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

        var list = findChild(page, "libraryGroupedListView")
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

    function playbackRowsPerGroup() {
        const rows = []
        for (let i = 0; i < 30; ++i)
            rows.push(groupedRow("today-" + i, "today"))
        for (let i = 0; i < 30; ++i)
            rows.push(groupedRow("yesterday-" + i, "yesterday"))
        for (let i = 0; i < 30; ++i)
            rows.push(groupedRow("earlier-" + i, "earlier"))
        return rows
    }

    /// 右键空白处时菜单必须贴着光标。回归的是 TapHandler.point.position
    /// 用内容坐标直接 mapToItem 的老问题（docs/pitfalls.md 第 44 条）：
    /// 滚动越深偏移越大，最后被弹窗定位器夹到窗口底部（状态条附近）。
    function test_playback_history_list_blank_menu_follows_cursor_after_scroll() {
        backend.libraryKind = "playback-history"
        page.kind = "playback-history"
        page.dateSectionsEnabled = true
        publish(playbackRowsPerGroup(), true)

        var list = findChild(page, "libraryGroupedListView")
        verify(list !== null)
        list.positionViewAtIndex(60, ListView.Beginning)
        wait(50)
        var earlierHeader = findChild(list, "libraryDateSection_earlier")
        verify(earlierHeader !== null, "更早分区头必须已实例化")
        const headerContentY = earlierHeader.mapToItem(list, 0, 0).y + list.contentY
        list.contentY = Math.max(0, headerContentY - 120)
        wait(50)

        const clickX = 100
        const clickY = earlierHeader.mapToItem(list, 0, earlierHeader.height / 2).y
        verify(clickY > 0 && clickY < list.height, "更早分区头应滚到可见区域")
        const expected = list.mapToItem(null, clickX, clickY)
        mouseClick(list, clickX, clickY, Qt.RightButton)
        tryCompare(page.blankContextMenu, "visible", true)
        compare(Math.round(page.blankContextMenu.x), Math.round(expected.x),
                "列表空白菜单横向跟随光标")
        compare(Math.round(page.blankContextMenu.y), Math.round(expected.y),
                "列表空白菜单纵向跟随光标")
        page.blankContextMenu.close()
    }

    function test_library_source_hint_reports_index_or_fallback() {
        backend.libraryStatus = "来自本地索引（更新于 2026-11-04 10:00），未请求网盘"
        publish([row("indexed", false, "file", true)], true)
        const hint = findChild(page, "librarySourceHint")
        verify(hint !== null, "缺少数据来源提示")
        verify(hint.visible)
        verify(hint.text.indexOf("来自本地索引") >= 0)

        backend.libraryStatus = "本地索引尚未就绪，已从网盘读取"
        backend.libraryRevision += 1
        backend.changed()
        wait(20)
        verify(hint.text.indexOf("已从网盘读取") >= 0)
    }

    function test_playback_history_grouped_grid_blank_menu_follows_cursor_after_scroll() {
        backend.libraryKind = "playback-history"
        page.kind = "playback-history"
        page.dateSectionsEnabled = true
        publish(playbackRowsPerGroup(), true)

        page.setContentView("grid")
        var grouped = findChild(page, "libraryGroupedGrid")
        var groupedList = findChild(page, "libraryGroupedList")
        verify(grouped !== null && groupedList !== null)
        tryCompare(grouped, "visible", true)
        tryVerify(function () { return grouped.contentHeight > grouped.height })

        // 滚到「更早」分组，再点卡片下方 18px 的行间缝隙。
        grouped.positionViewAtIndex(60, ListView.Beginning)
        wait(50)
        tryVerify(function () { return grouped.itemAtIndex(60) !== null })
        const card = grouped.itemAtIndex(60)
        verify(card !== null, "更早分组必须有可见卡片")
        const clickX = card.mapToItem(groupedList, card.width / 2, 0).x
        const clickY = card.mapToItem(groupedList, 0, 0).y + card.height
                       + grouped.rowGap / 2
        verify(clickY > 0 && clickY < groupedList.height,
               "更早分组的行间缝隙应在可见区域")
        const expected = groupedList.mapToItem(null, clickX, clickY)
        mouseClick(groupedList, clickX, clickY, Qt.RightButton)
        tryCompare(page.blankContextMenu, "visible", true)
        compare(Math.round(page.blankContextMenu.x), Math.round(expected.x),
                "分区网格空白菜单横向跟随光标")
        compare(Math.round(page.blankContextMenu.y), Math.round(expected.y),
                "分区网格空白菜单纵向跟随光标")
        page.blankContextMenu.close()
    }
}
