import QtQuick
import QtTest
import ".."

// 文件类内容页网格基础组件回归：
// - 网格使用 3:2 缩略图，普通目录只显示最多两行文件名；
// - 类型 / 大小 / 修改时间不占卡片文字区，违规状态留在缩略图角标；
// - 列表 / 网格切换沿用同一份文件和选择模型；
// - 两种视图各自记住滚动位置，切换回来不跳顶部。
TestCase {
    name: "FileGrid"
    when: windowShown
    width: 760
    height: 560

    KTheme {
        id: themeProbe
    }

    UiMetrics {
        id: uiMetrics
    }

    QtObject {
        id: fakeBackend

        property var files: []
        property var searchResults: []
        property string contentView: "list"
        property bool searchActive: false
        property bool searchSettled: true
        property bool searchTruncated: false
        property string searchStatus: ""
        property string searchQuery: ""
        property bool filesLoading: false
        property bool filesRefreshing: false
        property bool filesStale: false
        property int filesRevision: 1
        property var breadcrumb: ["全部文件"]
        property var playMenu: []
        property string sortField: "name"
        property bool sortAscending: true
        property bool fileActionRunning: false
        property bool shareActionRunning: false
        property var moveTargets: []
        property bool moveTargetsLoading: false
        property int moveTargetsRevision: 0
        property string moveTargetsError: ""
        property string shareCreatedUrl: ""
        property string shareCreatedPassCode: ""
        property string shareCreatedExpiration: ""
        property int thumbnailRequests: 0
        property bool playLoading: false
        property string playLoadingId: ""
        property var playingIds: []
        property int cancelPlayCalls: 0
        property int newFolderCalls: 0
        property int refreshCalls: 0
        property int searchOpenCalls: 0
        property int searchRevealCalls: 0

        signal changed()
        signal fileActionDone(string action, bool ok, string message)
        signal shareActionDone(string action, bool ok, string message)
        signal thumbnailReady(string id, string path)

        function thumbnailPath(id) {
            return ""
        }

        function requestThumbnail(id) {
            thumbnailRequests += 1
        }

        function cancelPlayLoading() {
            cancelPlayCalls += 1
            playLoading = false
            playLoadingId = ""
        }

        function setContentView(view) {
            contentView = view === "grid" ? "grid" : "list"
        }

        function openDir(id) {}
        function openParent() {}
        function openCrumb(index) {}
        function clearStatusMessage() {}
        function refreshFiles() {
            refreshCalls += 1
        }
        function beginNewFolder() {
            newFolderCalls += 1
        }
        function exitSearch() {}
        function searchLocal(text) {}
        function searchAll() {}
        function stopSearch() {}
        function searchOpenFolder(id) {
            searchOpenCalls += 1
            searchActive = false
            searchResults = []
            searchStatus = ""
            searchTruncated = false
            filesLoading = true
            changed()
        }
        function searchRevealFolder(id) {
            searchRevealCalls += 1
        }
        function requestPlay(id) {}
        function downloadEntry(id) {}
        function isDownloadPending(id) { return false }
        function hasPartialDownload(id) { return false }
        function setSorting(field, ascending) {}
        function currentPath() { return [{ id: "", name: "全部文件" }] }
        function loadMoveTargets(id) {}
        function cancelMoveTargets() {}
    }

    Component {
        id: densityComponent

        FileListDensity {}
    }

    Component {
        id: gridItemComponent

        FileGridItem {
            index: -1
            backend: fakeBackend
            playLoading: fakeBackend.playLoading
                         && fakeBackend.playLoadingId === (entry ? entry.id : "")
        }
    }

    Component {
        id: pageComponent

        FilesPage {
            width: 620
            height: 500
            backend: fakeBackend
        }
    }

    Component {
        id: searchBarComponent

        FilesPageSearchBar {
            width: 360
            height: 34
            backend: fakeBackend
        }
    }

    Component {
        id: viewToggleComponent

        ContentViewToggle {
            width: 100
            height: 34
            backend: fakeBackend
        }
    }

    Component {
        id: headerActionsComponent

        FilesPageHeaderActions {
            width: 280
            height: 34
            backend: fakeBackend
            target: fakeBackend
        }
    }

    property var density: null
    property var gridItem: null
    property var page: null
    property var searchBar: null
    property var viewToggle: null
    property var headerActions: null

    function entry(index, kind) {
        return {
            id: "entry-" + index,
            name: kind === "folder" ? "目录 " + index : "第 " + index + " 集.mkv",
            kind: kind,
            sizeText: kind === "folder" ? "—" : "1.2 GB",
            modifiedText: "2026-10-30 10:00",
            hasThumbnail: false,
            video: kind === "file",
            playable: kind === "file",
            auditFlag: false,
            starred: false
        }
    }

    function manyEntries(count) {
        const list = []
        for (let i = 0; i < count; i++)
            list.push(entry(i, i % 5 === 0 ? "folder" : "file"))
        return list
    }

    function cleanup() {
        if (page)
            page.destroy()
        if (gridItem)
            gridItem.destroy()
        if (density)
            density.destroy()
        if (searchBar)
            searchBar.destroy()
        if (viewToggle)
            viewToggle.destroy()
        if (headerActions)
            headerActions.destroy()
        page = null
        gridItem = null
        density = null
        searchBar = null
        viewToggle = null
        headerActions = null
        fakeBackend.files = []
        fakeBackend.searchResults = []
        fakeBackend.contentView = "list"
        fakeBackend.searchActive = false
        fakeBackend.searchSettled = true
        fakeBackend.searchTruncated = false
        fakeBackend.searchStatus = ""
        fakeBackend.searchQuery = ""
        fakeBackend.filesLoading = false
        fakeBackend.filesRefreshing = false
        fakeBackend.filesRevision = 1
        fakeBackend.thumbnailRequests = 0
        fakeBackend.playLoading = false
        fakeBackend.playLoadingId = ""
        fakeBackend.playingIds = []
        fakeBackend.cancelPlayCalls = 0
        fakeBackend.newFolderCalls = 0
        fakeBackend.refreshCalls = 0
        fakeBackend.searchOpenCalls = 0
        fakeBackend.searchRevealCalls = 0
    }

    function test_grid_item_uses_three_by_two_thumbnail_and_two_line_name() {
        density = densityComponent.createObject(parent)
        verify(density !== null)
        const width = 200
        const height = density.gridItemHeight(width)
        const expectedRadius = density.gridThumbnailRadius(width)
        compare(density.gridThumbnailRadius(900), density.gridThumbnailMaxRadius)
        verify(expectedRadius >= density.gridThumbnailMinRadius)
        const folder = entry(1, "folder")
        folder.name = "这是一个用于验证网格文件名最多显示两行且第二行末尾会省略的超长文件夹名称"
        gridItem = gridItemComponent.createObject(parent, {
            width: width,
            height: height,
            contentPadding: density.gridCardPadding,
            cardSpacing: density.gridCardSpacing,
            nameLineHeight: density.gridNameLineHeight,
            nameBlockHeight: density.gridNameBlockHeight,
            thumbnailRadius: expectedRadius,
            textPixelSize: density.namePixelSize,
            iconPixelSize: density.gridIconPixelSize,
            videoBadgeSize: density.gridVideoBadgeSize,
            entry: folder
        })
        verify(gridItem !== null)
        wait(30)

        const thumbnail = findChild(gridItem, "gridThumbnail")
        const fallback = findChild(gridItem, "thumbnailFallbackIcon")
        const thumbnailFrame = findChild(gridItem, "gridThumbnailFrame")
        const selectionRing = findChild(gridItem, "gridSelectionRing")
        const maskShape = findChild(thumbnail, "thumbnailMaskShape")
        const name = findChild(gridItem, "gridName")
        const auditBadge = findChild(gridItem, "gridAuditBadge")
        verify(thumbnail !== null && fallback !== null && thumbnailFrame !== null
               && selectionRing !== null && maskShape !== null && name !== null
               && auditBadge !== null)
        compare(thumbnail.width, width - density.gridCardPadding * 2)
        compare(thumbnail.height, Math.ceil(thumbnail.width * 2 / 3))
        compare(thumbnail.radius, expectedRadius)
        compare(thumbnailFrame.radius, expectedRadius)
        compare(selectionRing.radius, expectedRadius)
        compare(maskShape.radius, thumbnail.radius)
        compare(maskShape.width, thumbnail.width)
        compare(maskShape.height, thumbnail.height)
        compare(fallback.visible, true)
        compare(name.text, folder.name)
        compare(name.wrapMode, Text.WrapAnywhere)
        compare(name.maximumLineCount, 2)
        compare(name.elide, Text.ElideRight)
        compare(name.lineCount, 2)
        compare(name.height, density.gridNameBlockHeight)
        compare(findChild(gridItem, "gridMeta"), null, "网格不再显示类型 / 时间 / 大小")
        compare(auditBadge.visible, false)
        compare(fakeBackend.thumbnailRequests, 0, "文件夹不应请求缩略图")
    }

    function test_grid_thumbnail_decode_size_does_not_follow_card_width() {
        density = densityComponent.createObject(parent)
        verify(density !== null)
        const initialWidth = 180
        gridItem = gridItemComponent.createObject(parent, {
            width: initialWidth,
            height: density.gridItemHeight(initialWidth),
            contentPadding: density.gridCardPadding,
            cardSpacing: density.gridCardSpacing,
            nameLineHeight: density.gridNameLineHeight,
            nameBlockHeight: density.gridNameBlockHeight,
            thumbnailRadius: density.gridThumbnailRadius(initialWidth),
            thumbnailDecodeWidth: density.gridThumbnailDecodeWidth,
            thumbnailDecodeHeight: density.gridThumbnailDecodeHeight,
            entry: entry(10, "file")
        })
        verify(gridItem !== null)
        wait(20)

        const thumbnail = findChild(gridItem, "gridThumbnail")
        verify(thumbnail !== null)
        const image = findChild(thumbnail, "thumbnailImage")
        verify(image !== null)
        compare(image.sourceSize.width, density.gridThumbnailDecodeWidth)
        compare(image.sourceSize.height, density.gridThumbnailDecodeHeight)

        const resizedWidth = 260
        gridItem.width = resizedWidth
        gridItem.height = density.gridItemHeight(resizedWidth)
        wait(20)

        compare(thumbnail.width, resizedWidth - density.gridCardPadding * 2)
        compare(image.sourceSize.width, density.gridThumbnailDecodeWidth,
                "卡片变宽时不能改变缩略图解码宽度")
        compare(image.sourceSize.height, density.gridThumbnailDecodeHeight,
                "卡片变宽时不能改变缩略图解码高度")
    }

    function test_grid_video_loading_centers_spinner_and_cancels_without_selection() {
        density = densityComponent.createObject(parent)
        verify(density !== null)
        const video = entry(4, "file")
        fakeBackend.playLoading = true
        fakeBackend.playLoadingId = video.id
        gridItem = gridItemComponent.createObject(parent, {
            width: 200,
            height: density.gridItemHeight(200),
            contentPadding: density.gridCardPadding,
            cardSpacing: density.gridCardSpacing,
            nameLineHeight: density.gridNameLineHeight,
            nameBlockHeight: density.gridNameBlockHeight,
            thumbnailRadius: density.gridThumbnailRadius(200),
            entry: video
        })
        verify(gridItem !== null)
        wait(30)

        const overlay = findChild(gridItem, "gridVideoLoading")
        verify(overlay !== null)
        const scrim = findChild(overlay, "videoLoadingScrim")
        const spinner = findChild(overlay, "videoLoadingSpinner")
        const cancel = findChild(overlay, "videoLoadingCancel")
        verify(scrim !== null && spinner !== null && cancel !== null)
        compare(overlay.visible, true)
        compare(scrim.radius, density.gridThumbnailRadius(200))
        compare(scrim.color, themeProbe.mediaScrim)
        compare(themeProbe.mediaForeground, "#ffffff")
        compare(spinner.color, "#ffffff")
        compare(spinner.width, 28)
        const thumbnail = findChild(gridItem, "gridThumbnail")
        compare(findChild(thumbnail, "videoBadge").visible, false)
        const spinnerCenter = spinner.mapToItem(overlay, spinner.width / 2,
                                                spinner.height / 2)
        fuzzyCompare(spinnerCenter.x, overlay.width / 2, 1)
        fuzzyCompare(spinnerCenter.y, overlay.height / 2, 1)
        compare(findChild(gridItem, "gridMediaStateChip"), null)

        const spy = Qt.createQmlObject(
            "import QtTest; SignalSpy { signalName: 'selectRequested' }",
            gridItem)
        spy.target = gridItem
        mouseClick(cancel)
        compare(fakeBackend.cancelPlayCalls, 1)
        compare(spy.count, 0, "取消加载不能顺带选中视频")
        tryCompare(overlay, "visible", false)
        spy.destroy()
    }

    function test_grid_playing_video_uses_ring_and_pill() {
        density = densityComponent.createObject(parent)
        verify(density !== null)
        const video = entry(6, "file")
        gridItem = gridItemComponent.createObject(parent, {
            width: 200,
            height: density.gridItemHeight(200),
            contentPadding: density.gridCardPadding,
            cardSpacing: density.gridCardSpacing,
            nameLineHeight: density.gridNameLineHeight,
            nameBlockHeight: density.gridNameBlockHeight,
            thumbnailRadius: density.gridThumbnailRadius(200),
            entry: video,
            playing: true
        })
        verify(gridItem !== null)
        wait(30)

        const thumbnail = findChild(gridItem, "gridThumbnail")
        const thumbnailBox = findChild(gridItem, "gridThumbnailBox")
        const ring = findChild(thumbnail, "playingRing")
        const pill = findChild(gridItem, "gridPlayingPill")
        verify(thumbnail !== null && thumbnailBox !== null && ring !== null && pill !== null)
        compare(thumbnail.playing, true)
        compare(findChild(thumbnail, "videoBadge").visible, false)
        compare(ring.visible, true)
        compare(ring.border.width, 2)
        compare(pill.visible, true)
        compare(pill.parent, thumbnailBox)
        compare(gridItem.background.color, themeProbe.accentGhost)

        gridItem.playLoading = true
        tryCompare(pill, "visible", false)
        compare(findChild(gridItem, "gridVideoLoading").visible, true)
        compare(thumbnail.playing, false)
        compare(findChild(thumbnail, "videoBadge").visible, false)
    }

    function test_grid_playable_video_has_only_the_badge() {
        density = densityComponent.createObject(parent)
        const video = entry(8, "file")
        gridItem = gridItemComponent.createObject(parent, {
            width: 200,
            height: density.gridItemHeight(200),
            contentPadding: density.gridCardPadding,
            cardSpacing: density.gridCardSpacing,
            nameLineHeight: density.gridNameLineHeight,
            nameBlockHeight: density.gridNameBlockHeight,
            thumbnailRadius: density.gridThumbnailRadius(200),
            entry: video
        })
        verify(gridItem !== null)
        wait(30)

        const badge = findChild(findChild(gridItem, "gridThumbnail"), "videoBadge")
        verify(badge !== null)
        compare(findChild(gridItem, "gridMediaStateChip"), null)
        compare(badge.visible, true)
        compare(badge.radius, badge.width / 2)

        gridItem.entry = entry(9, "folder")
        tryCompare(badge, "visible", false)
    }

    function test_grid_item_marks_audit_files_on_the_thumbnail() {
        density = densityComponent.createObject(parent)
        const audited = entry(3, "file")
        audited.auditFlag = true
        gridItem = gridItemComponent.createObject(parent, {
            width: 180,
            height: density.gridItemHeight(180),
            contentPadding: density.gridCardPadding,
            cardSpacing: density.gridCardSpacing,
            nameLineHeight: density.gridNameLineHeight,
            nameBlockHeight: density.gridNameBlockHeight,
            thumbnailRadius: density.gridThumbnailRadius(180),
            entry: audited
        })
        verify(gridItem !== null)
        wait(20)

        const badge = findChild(gridItem, "gridAuditBadge")
        const thumbnailBox = findChild(gridItem, "gridThumbnailBox")
        verify(badge !== null && thumbnailBox !== null)
        compare(badge.visible, true)
        compare(badge.parent, thumbnailBox)
    }

    function test_grid_star_badge_does_not_change_card_geometry() {
        density = densityComponent.createObject(parent)
        const starred = entry(8, "file")
        starred.starred = true
        gridItem = gridItemComponent.createObject(parent, {
            width: 200,
            height: density.gridItemHeight(200),
            contentPadding: density.gridCardPadding,
            cardSpacing: density.gridCardSpacing,
            nameLineHeight: density.gridNameLineHeight,
            nameBlockHeight: density.gridNameBlockHeight,
            thumbnailRadius: density.gridThumbnailRadius(200),
            entry: starred
        })
        verify(gridItem !== null)
        wait(30)

        const badge = findChild(gridItem, "gridStarBadge")
        const star = findChild(gridItem, "gridStar")
        verify(badge !== null && star !== null)
        compare(badge.visible, true)
        compare(star.filled, true)
        compare(star.color, themeProbe.accent)
        const cardWidth = gridItem.width
        const cardHeight = gridItem.height
        const thumbnail = findChild(gridItem, "gridThumbnail")
        const thumbnailWidth = thumbnail.width
        const thumbnailHeight = thumbnail.height

        gridItem.entry = entry(9, "file")
        wait(20)
        compare(badge.visible, false)
        compare(gridItem.width, cardWidth)
        compare(gridItem.height, cardHeight)
        compare(thumbnail.width, thumbnailWidth)
        compare(thumbnail.height, thumbnailHeight)
    }

    function test_selected_grid_item_has_accent_ring() {
        density = densityComponent.createObject(parent)
        gridItem = gridItemComponent.createObject(parent, {
            width: 180,
            height: density.gridItemHeight(180),
            contentPadding: density.gridCardPadding,
            cardSpacing: density.gridCardSpacing,
            nameLineHeight: density.gridNameLineHeight,
            nameBlockHeight: density.gridNameBlockHeight,
            thumbnailRadius: density.gridThumbnailRadius(180),
            entry: entry(2, "file")
        })
        verify(gridItem !== null)
        wait(20)

        const ring = findChild(gridItem, "gridSelectionRing")
        verify(ring !== null)
        compare(ring.border.width, 0)
        gridItem.selected = true
        tryVerify(function () {
            return ring.border.width === 2
        })
    }

    function test_page_switches_between_list_and_grid_and_keeps_scroll() {
        fakeBackend.files = manyEntries(30)
        fakeBackend.contentView = "list"
        page = pageComponent.createObject(parent, { visible: true })
        verify(page !== null)
        wait(50)

        const list = findChild(page, "fileList")
        const grid = findChild(page, "fileGrid")
        verify(list !== null && grid !== null)
        compare(list.visible, true)
        compare(grid.visible, false)
        list.contentY = 48
        wait(30)

        page.setContentView("grid")
        wait(50)
        compare(fakeBackend.contentView, "grid")
        compare(list.visible, false)
        compare(grid.visible, true)
        grid.contentY = 96
        wait(30)

        page.setContentView("list")
        wait(50)
        compare(fakeBackend.contentView, "list")
        compare(list.visible, true)
        compare(grid.visible, false)
        tryCompare(list, "contentY", 48)
    }

    function test_file_grid_shows_size_or_folder_and_modified_time() {
        fakeBackend.files = [entry(1, "folder"), entry(2, "file")]
        fakeBackend.contentView = "grid"
        page = pageComponent.createObject(parent, { visible: true })
        verify(page !== null)
        wait(80)

        const grid = findChild(page, "fileGrid")
        verify(grid !== null)
        const folderCard = grid.itemAtIndex(0)
        const fileCard = grid.itemAtIndex(1)
        verify(folderCard !== null && fileCard !== null)

        const folderDetail = findChild(folderCard, "gridDetail")
        const fileDetail = findChild(fileCard, "gridDetail")
        verify(folderDetail !== null && fileDetail !== null)
        compare(folderDetail.visible, true)
        compare(fileDetail.visible, true)
        compare(folderDetail.text, qsTr("文件夹 · 26-10-30 10:00"))
        compare(fileDetail.text, qsTr("1.2 GB · 26-10-30 10:00"))
        compare(folderCard.height, page.gridItemHeight)
        compare(fileCard.height, page.gridItemHeight)
    }

    function test_global_view_change_keeps_files_grid_scroll_while_page_is_hidden() {
        fakeBackend.files = manyEntries(30)
        fakeBackend.contentView = "list"
        page = pageComponent.createObject(parent, { visible: true })
        verify(page !== null)
        wait(50)

        const list = findChild(page, "fileList")
        const grid = findChild(page, "fileGrid")
        verify(list !== null && grid !== null)
        list.contentY = 48
        wait(30)

        page.setContentView("grid")
        wait(50)
        grid.contentY = 96
        wait(30)
        page.setContentView("list")
        wait(50)
        tryCompare(list, "contentY", 48)

        page.visible = false
        wait(30)
        fakeBackend.contentView = "grid"
        wait(50)
        page.visible = true
        tryCompare(grid, "contentY", 96)
    }

    function test_page_binds_multiple_playing_ids_to_rows() {
        fakeBackend.files = manyEntries(4)
        fakeBackend.playingIds = ["entry-0", "entry-2"]
        fakeBackend.contentView = "list"
        page = pageComponent.createObject(parent, { visible: true })
        verify(page !== null)
        wait(80)

        const list = findChild(page, "fileList")
        verify(list !== null)
        compare(page.isEntryPlaying("entry-0"), true)
        compare(page.isEntryPlaying("entry-1"), false)
        compare(page.isEntryPlaying("entry-2"), true)
        const first = list.itemAtIndex(0)
        const second = list.itemAtIndex(1)
        const third = list.itemAtIndex(2)
        verify(first !== null && second !== null && third !== null)
        compare(first.playing, true)
        compare(second.playing, false)
        compare(third.playing, true)

        fakeBackend.playingIds = ["entry-1"]
        tryCompare(first, "playing", false)
        tryCompare(second, "playing", true)
        tryCompare(third, "playing", false)
    }

    function test_reveal_entry_selects_and_scrolls_the_target() {
        fakeBackend.files = manyEntries(30)
        fakeBackend.contentView = "list"
        page = pageComponent.createObject(parent, { visible: true })
        verify(page !== null)
        wait(80)

        const list = findChild(page, "fileList")
        verify(list !== null)
        page.revealEntryById("entry-20")
        tryVerify(function () {
            const row = list.itemAtIndex(20)
            return row !== null && row.highlighted === true
        })
        compare(list.currentIndex, 20)
        verify(list.contentY > 0, "目标不在首屏时必须滚动到目标")

        page.setContentView("grid")
        wait(80)
        const grid = findChild(page, "fileGrid")
        verify(grid !== null)
        page.revealEntryById("entry-20")
        tryVerify(function () {
            const card = grid.itemAtIndex(20)
            return card !== null && card.selected === true
        })
        compare(grid.currentIndex, 20)
        verify(grid.contentY > 0, "网格视图也必须滚动到目标")
    }

    function test_grid_layout_fills_width_without_a_phantom_column() {
        fakeBackend.files = manyEntries(30)
        fakeBackend.contentView = "grid"
        page = pageComponent.createObject(parent, {
            width: 716,
            height: 560,
            visible: true
        })
        verify(page !== null)
        wait(60)

        const grid = findChild(page, "fileGrid")
        verify(grid !== null)
        const usableWidth = grid.width - grid.leftMargin - grid.rightMargin
        compare(grid.leftMargin, page.gridSideMargin)
        compare(grid.rightMargin, page.gridRightMargin)
        verify(page.gridColumns > 1)
        const firstItem = grid.itemAtIndex(0)
        verify(firstItem !== null)
        let actualColumns = 0
        for (let i = 0; i < page.gridColumns; i++) {
            const item = grid.itemAtIndex(i)
            if (!item || Math.abs(item.y - firstItem.y) > 0.5)
                break
            actualColumns += 1
        }
        compare(actualColumns, page.gridColumns, "实际列数不能比计算值少一列")
        const pageDensity = findChild(page, "fileListDensity")
        verify(pageDensity !== null)
        compare(grid.scrollBarVisible, true)
        verify(grid.contentHeight > grid.height)
        compare(page.gridCellSlotWidth, Math.floor(usableWidth / page.gridColumns))
        const occupiedWidth = (page.gridColumns - 1) * page.gridCellSlotWidth
                              + page.gridItemWidth
        verify(occupiedWidth <= usableWidth)
        const visualRightMargin = page.gridRightMargin
                                  + usableWidth - occupiedWidth
        const expectedRightMargin = page.gridSideMargin + uiMetrics.scrollBarSlot
        verify(visualRightMargin >= expectedRightMargin)
        verify(visualRightMargin - expectedRightMargin < page.gridColumns,
               "末列尾间距只能吸收取整误差，不能多留一整段列间距")
        const lastInRow = grid.itemAtIndex(page.gridColumns - 1)
        verify(lastInRow !== null)
        // GridView 委托的 x 从内容原点算起，leftMargin 不在委托坐标里。
        const actualRightMargin = grid.width - grid.leftMargin
                                  - lastInRow.x - lastInRow.width
        verify(actualRightMargin >= expectedRightMargin)
        verify(actualRightMargin - expectedRightMargin < page.gridColumns,
               "实际末列右缘不能多留一整段列间距（actual="
               + actualRightMargin + ", expected=" + expectedRightMargin
               + ", columns=" + page.gridColumns + "）")
        verify(page.gridItemWidth + pageDensity.gridColumnGap <= page.gridCellSlotWidth)
        const firstThumbnail = findChild(firstItem, "gridThumbnail")
        verify(firstThumbnail !== null)
        compare(firstThumbnail.radius,
                pageDensity.gridThumbnailRadius(page.gridItemWidth))
    }

    function test_grid_drops_scrollbar_lane_when_content_fits() {
        fakeBackend.files = manyEntries(8)
        fakeBackend.contentView = "grid"
        page = pageComponent.createObject(parent, {
            width: 716,
            height: 800,
            visible: true
        })
        verify(page !== null)
        wait(60)

        const grid = findChild(page, "fileGrid")
        verify(grid !== null)
        compare(grid.scrollBarVisible, false)
        verify(grid.contentHeight <= grid.height)
        const lastInRow = grid.itemAtIndex(page.gridColumns - 1)
        verify(lastInRow !== null)
        const actualRightMargin = grid.width - grid.leftMargin
                                  - lastInRow.x - lastInRow.width
        compare(actualRightMargin, page.gridSideMargin,
                "无滚动条时左右页面边距必须一致（actual=" + actualRightMargin
                + ", columns=" + page.gridColumns + ", x=" + lastInRow.x
                + ", y=" + lastInRow.y
                + ", width=" + lastInRow.width + ", grid=" + grid.width
                + ", margins=" + grid.leftMargin + "/" + grid.rightMargin
                + ", slot=" + page.gridCellSlotWidth
                + ", cell=" + grid.cellWidth
                + ", item0=" + grid.itemAtIndex(0).x + "/"
                + grid.itemAtIndex(0).y + ", item1=" + grid.itemAtIndex(1).x
                + "/" + grid.itemAtIndex(1).y + ", item2="
                + grid.itemAtIndex(2).x + "/" + grid.itemAtIndex(2).y + "）")
    }

    function test_search_placeholder_explains_scope_and_enter_action() {
        searchBar = searchBarComponent.createObject(parent, { visible: true })
        verify(searchBar !== null)
        wait(20)

        const field = findChild(searchBar, "filesSearchField")
        verify(field !== null)
        compare(field.placeholderText, "搜索当前文件夹，回车全盘搜索")
    }

    function test_view_toggle_switches_with_one_click_and_stays_available_during_search() {
        fakeBackend.contentView = "list"
        viewToggle = viewToggleComponent.createObject(parent, { visible: true })
        verify(viewToggle !== null)
        wait(30)

        const button = viewToggle
        compare(button.objectName, "viewToggleButton")
        compare(button.text, qsTr("列表"))
        compare(button.enabled, true)

        const spy = Qt.createQmlObject(
            "import QtTest; SignalSpy { signalName: 'viewRequested' }",
            viewToggle)
        spy.target = viewToggle
        mouseClick(button)
        wait(30)
        compare(spy.count, 1)
        compare(spy.signalArguments[0][0], "grid")

        fakeBackend.contentView = "grid"
        fakeBackend.searchActive = true
        compare(button.text, qsTr("网格"))
        compare(button.enabled, true)
        spy.destroy()
    }

    function test_search_results_reuse_the_current_file_view() {
        const result = entry(2, "file")
        result.folderText = "资料/剧集"
        result.folderId = "folder-2"
        fakeBackend.files = [entry(1, "folder")]
        fakeBackend.searchResults = [result]
        fakeBackend.searchActive = true
        fakeBackend.searchSettled = true
        fakeBackend.searchStatus = "搜索完成"
        fakeBackend.searchQuery = "第 2"
        fakeBackend.contentView = "list"
        page = pageComponent.createObject(parent, { visible: true })
        verify(page !== null)
        wait(80)

        const list = findChild(page, "fileList")
        const grid = findChild(page, "fileGrid")
        verify(list !== null && grid !== null)
        compare(list.visible, true)
        compare(grid.visible, false)
        compare(list.count, 1)
        const listRow = list.itemAtIndex(0)
        verify(listRow !== null)
        const detail = findChild(listRow, "fileRowDetail")
        verify(detail !== null)
        compare(detail.visible, true)
        verify(detail.text.indexOf("资料/剧集") >= 0)
        const detailPoint = detail.mapToItem(listRow, detail.width / 2,
                                             detail.height / 2)
        mouseClick(listRow, detailPoint.x, detailPoint.y)
        compare(fakeBackend.searchRevealCalls, 1)

        page.setContentView("grid")
        wait(80)
        compare(fakeBackend.contentView, "grid")
        compare(list.visible, false)
        compare(grid.visible, true)
        compare(grid.count, 1)
        const gridCard = grid.itemAtIndex(0)
        verify(gridCard !== null)
        const gridDetail = findChild(gridCard, "gridDetail")
        verify(gridDetail !== null)
        compare(gridDetail.visible, true)
        verify(gridDetail.text.indexOf("资料/剧集") >= 0)
        const gridDetailPoint = gridDetail.mapToItem(gridCard, gridDetail.width / 2,
                                                     gridDetail.height / 2)
        mouseClick(gridCard, gridDetailPoint.x, gridDetailPoint.y)
        compare(fakeBackend.searchRevealCalls, 2)
    }

    function test_search_state_change_does_not_steal_keyboard_focus() {
        fakeBackend.files = [entry(1, "folder")]
        fakeBackend.searchResults = [entry(2, "file")]
        fakeBackend.searchActive = false
        page = pageComponent.createObject(parent, { visible: true })
        verify(page !== null)
        wait(60)

        const probe = Qt.createQmlObject(
            "import QtQuick.Controls; TextField { objectName: 'focusProbe'; width: 120; height: 32 }",
            page)
        verify(probe !== null)
        probe.forceActiveFocus()
        tryVerify(function () {
            return probe.activeFocus
        })

        fakeBackend.searchActive = true
        wait(40)
        compare(probe.activeFocus, true, "进入搜索不能抢走搜索框焦点")

        fakeBackend.searchActive = false
        wait(40)
        compare(probe.activeFocus, true, "清空搜索回目录时不能抢走搜索框焦点")
    }

    function test_search_folder_navigation_holds_results_until_target_loads() {
        const previous = entry(1, "folder")
        const target = entry(2, "folder")
        target.folderText = "全部文件"
        target.folderId = ""
        fakeBackend.files = [previous]
        fakeBackend.searchResults = [target]
        fakeBackend.searchActive = true
        fakeBackend.searchSettled = true
        fakeBackend.searchStatus = "搜索完成"
        fakeBackend.searchQuery = "目录 2"
        fakeBackend.contentView = "list"
        page = pageComponent.createObject(parent, { visible: true })
        verify(page !== null)
        wait(80)

        const list = findChild(page, "fileList")
        const overlay = findChild(page, "searchNavigationOverlay")
        const statusBar = findChild(page, "searchStatusBar")
        const statusText = findChild(page, "searchStatusText")
        verify(list !== null && overlay !== null)
        verify(statusBar !== null && statusText !== null)
        compare(list.count, 1)
        compare(list.itemAtIndex(0).entry.id, target.id)
        compare(overlay.visible, false)
        compare(statusBar.visible, true)
        compare(statusText.text, "搜索完成")

        page.openEntryFolder(target.id)
        wait(20)
        compare(fakeBackend.searchOpenCalls, 1)
        compare(page.holdingSearchEntries, true)
        compare(overlay.visible, true)
        compare(list.count, 1)
        compare(list.itemAtIndex(0).entry.id, target.id,
                "目标目录加载完成前必须保留搜索结果，不能闪回原目录")
        const heldRow = list.itemAtIndex(0)
        const heldDetail = findChild(heldRow, "fileRowDetail")
        verify(heldDetail !== null)
        compare(heldDetail.visible, true)
        compare(statusBar.visible, true)
        compare(statusText.text, "搜索完成",
                "目标目录加载完成前搜索状态文案必须保留")

        const loaded = entry(3, "file")
        fakeBackend.files = [loaded]
        fakeBackend.filesLoading = false
        fakeBackend.filesRevision += 1
        fakeBackend.changed()
        wait(40)

        compare(page.holdingSearchEntries, false)
        compare(overlay.visible, false)
        compare(statusBar.visible, false)
        compare(list.count, 1)
        compare(list.itemAtIndex(0).entry.id, loaded.id)
    }

    function test_header_actions_show_text_and_keep_refresh_last() {
        fakeBackend.contentView = "list"
        headerActions = headerActionsComponent.createObject(parent, { visible: true })
        verify(headerActions !== null)
        wait(30)

        const newFolder = findChild(headerActions, "filesNewFolderButton")
        const toggle = findChild(headerActions, "viewToggleButton")
        const refresh = findChild(headerActions, "filesRefreshButton")
        verify(newFolder !== null && toggle !== null && refresh !== null)
        compare(newFolder.text, qsTr("新建文件夹"))
        compare(toggle.text, qsTr("列表"))
        compare(refresh.text, qsTr("刷新"))
        verify(newFolder.x < toggle.x)
        verify(toggle.x < refresh.x, "刷新按钮应在最右侧")

        mouseClick(newFolder)
        wait(20)
        compare(fakeBackend.newFolderCalls, 1)

        mouseClick(toggle)
        wait(20)
        compare(fakeBackend.contentView, "grid")

        mouseClick(refresh)
        wait(20)
        compare(fakeBackend.refreshCalls, 1)
    }

    function test_refresh_button_feedback_is_manual_only() {
        fakeBackend.contentView = "list"
        fakeBackend.filesLoading = false
        fakeBackend.filesRefreshing = false
        headerActions = headerActionsComponent.createObject(parent, { visible: true })
        verify(headerActions !== null)
        wait(30)

        const refresh = findChild(headerActions, "filesRefreshButton")
        verify(refresh !== null)
        const spinner = findChild(refresh, "buttonBusyIndicator")
        verify(spinner !== null)
        compare(refresh.text, qsTr("刷新"))
        compare(refresh.enabled, true)
        compare(spinner.running, false)
        const idleWidth = refresh.width

        // TTL 后台校准不会置 loading / refreshing；按钮和文字必须保持原样。
        fakeBackend.filesRevision += 1
        fakeBackend.changed()
        wait(20)
        compare(refresh.text, qsTr("刷新"))
        compare(refresh.enabled, true)
        compare(spinner.running, false)

        fakeBackend.filesRefreshing = true
        fakeBackend.changed()
        wait(20)
        compare(refresh.text, qsTr("刷新"))
        compare(refresh.enabled, false)
        compare(spinner.running, true)
        compare(refresh.width, idleWidth)

        fakeBackend.filesRefreshing = false
        fakeBackend.changed()
        wait(20)
        compare(refresh.text, qsTr("刷新"))
        compare(refresh.enabled, true)
        compare(spinner.running, false)
        compare(refresh.width, idleWidth)
    }
}
