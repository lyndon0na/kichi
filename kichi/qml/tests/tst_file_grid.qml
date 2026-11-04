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

    QtObject {
        id: fakeBackend

        property var files: []
        property var searchResults: []
        property string authState: "signedIn"
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
        property string currentDirId: ""
        property var breadcrumb: ["全部文件"]
        property var playMenu: []
        property string sortField: "name"
        property bool sortAscending: true
        property bool fileActionRunning: false
        property var fileActionIds: []
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
        property int uploadCalls: 0
        property string uploadFolderId: ""
        property string uploadFolderName: ""
        property int uploadFolderCalls: 0
        property string uploadFolderFolderId: ""
        property string uploadFolderFolderName: ""
        property int droppedUploadCalls: 0
        property string droppedUploadFolderId: ""
        property string droppedUploadFolderName: ""
        property var droppedUploadUrls: []
        property int newTaskDialogCalls: 0
        property string newTaskDialogFolderId: ""
        property string newTaskDialogFolderPath: ""
        property var newTaskDialogStartPath: []
        property int refreshCalls: 0
        property int backgroundRefreshCalls: 0
        property int openDirCalls: 0
        property int searchOpenCalls: 0
        property int searchRevealCalls: 0

        signal changed()
        signal fileActionDone(string action, bool ok, string message, var ids)
        signal folderCreateStarted(string id)
        signal folderCreated(string id)
        signal offlineTaskFolderChanged(string folderId)
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

        function openDir(id) {
            openDirCalls += 1
        }
        function openParent() {}
        function openCrumb(index) {}
        function clearStatusMessage() {}
        function refreshFiles() {
            refreshCalls += 1
        }
        function refreshFilesBackground() {
            backgroundRefreshCalls += 1
        }
        function beginNewFolder() {
            newFolderCalls += 1
        }
        function chooseUploadFiles(folderId, folderName) {
            uploadCalls += 1
            uploadFolderId = String(folderId || "")
            uploadFolderName = String(folderName || "")
        }
        function chooseUploadFolders(folderId, folderName) {
            uploadFolderCalls += 1
            uploadFolderFolderId = String(folderId || "")
            uploadFolderFolderName = String(folderName || "")
        }
        function uploadDroppedFiles(folderId, folderName, urls) {
            droppedUploadCalls += 1
            droppedUploadFolderId = String(folderId || "")
            droppedUploadFolderName = String(folderName || "")
            droppedUploadUrls = urls.slice()
        }
        function openNewTaskDialog(folderId, folderPath, startPath) {
            newTaskDialogCalls += 1
            newTaskDialogFolderId = String(folderId || "")
            newTaskDialogFolderPath = String(folderPath || "")
            newTaskDialogStartPath = startPath ? startPath.slice() : []
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
            newTaskTarget: fakeBackend
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
        if (page) {
            page.activeFileView.cancelFlick()
            page.visible = false
            wait(50)
            page.destroy()
            wait(30)
        }
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
        fakeBackend.authState = "signedIn"
        fakeBackend.filesLoading = false
        fakeBackend.filesRefreshing = false
        fakeBackend.filesRevision = 1
        fakeBackend.currentDirId = ""
        fakeBackend.breadcrumb = ["全部文件"]
        fakeBackend.thumbnailRequests = 0
        fakeBackend.playLoading = false
        fakeBackend.playLoadingId = ""
        fakeBackend.playingIds = []
        fakeBackend.cancelPlayCalls = 0
        fakeBackend.newFolderCalls = 0
        fakeBackend.newTaskDialogCalls = 0
        fakeBackend.newTaskDialogFolderId = ""
        fakeBackend.newTaskDialogFolderPath = ""
        fakeBackend.newTaskDialogStartPath = []
        fakeBackend.uploadCalls = 0
        fakeBackend.uploadFolderId = ""
        fakeBackend.uploadFolderName = ""
        fakeBackend.uploadFolderCalls = 0
        fakeBackend.uploadFolderFolderId = ""
        fakeBackend.uploadFolderFolderName = ""
        fakeBackend.droppedUploadCalls = 0
        fakeBackend.droppedUploadFolderId = ""
        fakeBackend.droppedUploadFolderName = ""
        fakeBackend.droppedUploadUrls = []
        fakeBackend.refreshCalls = 0
        fakeBackend.backgroundRefreshCalls = 0
        fakeBackend.openDirCalls = 0
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

    function test_files_loading_uses_skeleton_for_each_empty_read() {
        fakeBackend.files = []
        fakeBackend.filesLoading = true
        page = pageComponent.createObject(parent, {
            visible: false
        })
        verify(page !== null)
        wait(20)

        page.visible = true
        wait(30)
        const skeleton = findChild(page, "pageLoadingSkeleton")
        const gate = findChild(page, "filesLoadingSkeletonGate")
        verify(skeleton !== null)
        verify(gate !== null)
        compare(skeleton.visible, true,
                "空内容目录读取显示骨架（page=" + page.showLoadingSkeleton
                + ", active=" + gate.active + ", loading=" + gate.loading
                + ", count=" + gate.count + "）")

        fakeBackend.filesLoading = false
        fakeBackend.files = manyEntries(2)
        fakeBackend.filesRevision += 1
        fakeBackend.changed()
        wait(30)
        compare(skeleton.visible, false, "读取结束收起骨架")

        fakeBackend.filesLoading = true
        fakeBackend.changed()
        wait(30)
        compare(skeleton.visible, true,
                "切到无缓存目录时，上一目录内容不挡住骨架")

        fakeBackend.files = []
        fakeBackend.filesLoading = false
        fakeBackend.filesRevision += 1
        fakeBackend.changed()
        wait(30)
        compare(skeleton.visible, false, "目录读取结束后收起骨架")
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
        compare(themeProbe.mediaForeground,
                themeProbe.darkMode ? themeProbe.palette.text : themeProbe.palette.base)
        compare(spinner.color, themeProbe.mediaForeground)
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

    function test_grid_audio_keeps_music_icon_and_play_badge() {
        density = densityComponent.createObject(parent)
        verify(density !== null)
        const audio = entry(10, "file")
        audio.name = "夜曲.flac"
        audio.video = false
        audio.playable = true
        gridItem = gridItemComponent.createObject(parent, {
            width: 200,
            height: density.gridItemHeight(200),
            contentPadding: density.gridCardPadding,
            cardSpacing: density.gridCardSpacing,
            nameLineHeight: density.gridNameLineHeight,
            nameBlockHeight: density.gridNameBlockHeight,
            thumbnailRadius: density.gridThumbnailRadius(200),
            entry: audio
        })
        verify(gridItem !== null)
        wait(30)

        const thumbnail = findChild(gridItem, "gridThumbnail")
        const icon = findChild(thumbnail, "thumbnailFallbackIcon")
        verify(icon !== null)
        compare(icon.iconName, "music")
        const badge = findChild(thumbnail, "videoBadge")
        verify(badge !== null)
        compare(badge.visible, true)
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
        compare(badge.color, themeProbe.mediaBadge)
        compare(star.filled, true)
        compare(star.color, themeProbe.mediaForeground)
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

    // 加载中的条目离开当前列表（切目录 / 退出搜索）后不再内联显示，
    // 底部提示条据此接管取消入口。
    function test_inline_play_loading_follows_visible_entries() {
        fakeBackend.files = [entry(0, "file"), entry(1, "file")]
        fakeBackend.contentView = "list"
        fakeBackend.playLoading = true
        fakeBackend.playLoadingId = "entry-0"
        page = pageComponent.createObject(parent, { visible: true })
        verify(page !== null)
        wait(30)
        compare(page.showsInlinePlayLoading, true)

        // 进入另一个目录：原来的条目不在列表里
        fakeBackend.files = [entry(2, "file")]
        fakeBackend.filesRevision += 1
        fakeBackend.currentDirId = "another-folder"
        fakeBackend.changed()
        tryCompare(page, "showsInlinePlayLoading", false)

        // 搜索结果里又能看到它：交回缩略图
        fakeBackend.searchActive = true
        fakeBackend.searchResults = [entry(0, "file")]
        fakeBackend.changed()
        tryCompare(page, "showsInlinePlayLoading", true)

        // 退出搜索，仍停在没有该条目的目录
        fakeBackend.searchActive = false
        fakeBackend.searchResults = []
        fakeBackend.changed()
        tryCompare(page, "showsInlinePlayLoading", false)
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

    function test_created_folder_is_revealed_without_entering_or_selecting() {
        const pendingEntries = manyEntries(30)
        pendingEntries[25] = entry(25, "folder")
        pendingEntries[25].id = "pending-folder-25"
        pendingEntries[25].name = "新建文件夹"
        fakeBackend.files = pendingEntries
        fakeBackend.contentView = "list"
        page = pageComponent.createObject(parent, { visible: true })
        verify(page !== null)
        wait(80)

        const list = findChild(page, "fileList")
        verify(list !== null)
        fakeBackend.folderCreateStarted("pending-folder-25")
        tryVerify(function () {
            const row = list.itemAtIndex(25)
            return row !== null && row.createdFlash === true
        })

        const row = list.itemAtIndex(25)
        verify(row !== null)
        const flash = findChild(row, "fileRowCreatedFlash")
        verify(flash !== null)
        tryVerify(function () {
            return flash.visible && flash.opacity > 0
        })
        compare(list.currentIndex, 25)
        verify(list.contentY > 0, "新文件夹不在首屏时必须滚动到它")
        compare(row.highlighted, false, "只强调，不改变当前选择")
        compare(fakeBackend.openDirCalls, 0, "不能进入新目录")
        compare(fakeBackend.currentDirId, "", "当前目录保持不变")

        const revealedY = list.contentY
        const resolvedEntries = manyEntries(30)
        resolvedEntries[25] = entry(25, "folder")
        resolvedEntries[25].name = "新建文件夹"
        fakeBackend.files = resolvedEntries
        fakeBackend.filesRevision += 1
        fakeBackend.changed()
        fakeBackend.folderCreated("entry-25")
        tryVerify(function () {
            const resolved = list.itemAtIndex(25)
            return resolved !== null && resolved.createdFlash === true
        })
        compare(list.currentIndex, 25)
        verify(Math.abs(list.contentY - revealedY) <= 1,
               "真实 id 接管时不能再次滚动")
        compare(fakeBackend.openDirCalls, 0, "真实 id 回来也不能进入目录")

        tryVerify(function () {
            const current = list.itemAtIndex(25)
            return current !== null && current.createdFlash === false
        }, 3000)
    }

    function test_created_folder_flash_works_in_grid_without_navigation() {
        const pendingEntries = manyEntries(30)
        pendingEntries[25] = entry(25, "folder")
        pendingEntries[25].id = "pending-folder-25"
        pendingEntries[25].name = "新建文件夹"
        fakeBackend.files = pendingEntries
        fakeBackend.contentView = "grid"
        page = pageComponent.createObject(parent, { visible: true })
        verify(page !== null)
        wait(80)

        const grid = findChild(page, "fileGrid")
        verify(grid !== null)
        fakeBackend.folderCreateStarted("pending-folder-25")
        tryVerify(function () {
            const card = grid.itemAtIndex(25)
            return card !== null && card.createdFlash === true
        })

        const card = grid.itemAtIndex(25)
        verify(card !== null)
        const flash = findChild(card, "gridCreatedFlash")
        verify(flash !== null)
        tryVerify(function () {
            return flash.visible && flash.opacity > 0
        })
        compare(grid.currentIndex, 25)
        verify(grid.contentY > 0, "网格也必须滚动到新文件夹")
        compare(card.selected, false, "只强调，不改变当前选择")
        compare(fakeBackend.openDirCalls, 0, "不能进入新目录")

        const revealedY = grid.contentY
        const resolvedEntries = manyEntries(30)
        resolvedEntries[25] = entry(25, "folder")
        resolvedEntries[25].name = "新建文件夹"
        fakeBackend.files = resolvedEntries
        fakeBackend.filesRevision += 1
        fakeBackend.changed()
        fakeBackend.folderCreated("entry-25")
        tryVerify(function () {
            const resolved = grid.itemAtIndex(25)
            return resolved !== null && resolved.createdFlash === true
        })
        verify(Math.abs(grid.contentY - revealedY) <= 1,
               "网格里真实 id 接管时也不能再次滚动")
        compare(fakeBackend.openDirCalls, 0, "真实 id 回来也不能进入目录")
    }

    function test_created_folder_failure_clears_flash() {
        const pendingEntries = manyEntries(30)
        pendingEntries[25] = entry(25, "folder")
        pendingEntries[25].id = "pending-folder-25"
        pendingEntries[25].name = "新建文件夹"
        fakeBackend.files = pendingEntries
        fakeBackend.contentView = "list"
        page = pageComponent.createObject(parent, { visible: true })
        verify(page !== null)
        wait(80)

        const list = findChild(page, "fileList")
        verify(list !== null)
        fakeBackend.folderCreateStarted("pending-folder-25")
        tryVerify(function () {
            const row = list.itemAtIndex(25)
            return row !== null && row.createdFlash === true
        })

        fakeBackend.fileActionDone("folder", false, "同名文件夹已存在", [])
        tryVerify(function () {
            const row = list.itemAtIndex(25)
            return row !== null && row.createdFlash === false
        })
        compare(fakeBackend.currentDirId, "", "失败也不能改变当前目录")
    }

    function test_list_first_click_after_flick_selects_and_double_click_opens() {
        fakeBackend.files = manyEntries(40)
        fakeBackend.contentView = "list"
        page = pageComponent.createObject(parent, { visible: true })
        verify(page !== null)
        wait(80)

        const list = findChild(page, "fileList")
        verify(list !== null)
        verify(list.contentHeight > list.height)
        const x = 120
        const y = list.height / 2

        list.flick(0, -600)
        wait(30)
        verify(list.moving, "测试必须覆盖惯性滚动期间")
        const index = list.indexAt(x, y + list.contentY)
        verify(index >= 0)
        mouseClick(list, x, y)
        wait(50)
        const row = list.itemAtIndex(index)
        verify(row !== null)
        compare(row.highlighted, true, "滚轮后第一次单击必须选中当前行")

        fakeBackend.openDirCalls = 0
        list.flick(0, -600)
        wait(30)
        verify(list.moving)
        let folderY = -1
        for (let scanY = 12; scanY < list.height - 12; scanY += 24) {
            const scanIndex = list.indexAt(x, scanY + list.contentY)
            const scanRow = list.itemAtIndex(scanIndex)
            if (scanRow && scanRow.entry && scanRow.entry.kind === "folder") {
                folderY = scanY
                break
            }
        }
        verify(folderY >= 0, "视口内必须能找到一个文件夹")
        mouseDoubleClickSequence(list, x, folderY)
        wait(50)
        compare(fakeBackend.openDirCalls, 1,
                "惯性期间的双击也必须一次打开文件夹")
    }

    function test_grid_first_click_after_flick_selects_and_double_click_opens() {
        fakeBackend.files = manyEntries(40)
        fakeBackend.contentView = "grid"
        page = pageComponent.createObject(parent, { visible: true })
        verify(page !== null)
        wait(80)

        const grid = findChild(page, "fileGrid")
        verify(grid !== null)
        verify(grid.contentHeight > grid.height)

        grid.flick(0, -600)
        wait(30)
        verify(grid.moving, "测试必须覆盖惯性滚动期间")
        const centerIndex = grid.indexAt(grid.width / 2,
                                         grid.height / 2 + grid.contentY)
        verify(centerIndex >= 0)
        const centerCard = grid.itemAtIndex(centerIndex)
        verify(centerCard !== null)
        const center = grid.mapFromItem(centerCard,
                                        centerCard.width / 2,
                                        centerCard.height / 2)
        mouseClick(grid, center.x, center.y)
        wait(50)
        verify(centerCard.selected, "滚轮后第一次单击必须选中当前卡片")

        fakeBackend.openDirCalls = 0
        grid.flick(0, -600)
        wait(30)
        verify(grid.moving)
        let folderPoint = null
        for (let i = 0; i < grid.count; i++) {
            const card = grid.itemAtIndex(i)
            if (!card || !card.entry || card.entry.kind !== "folder")
                continue
            const point = grid.mapFromItem(card, card.width / 2, card.height / 2)
            if (point.y >= 0 && point.y < grid.height
                    && point.x >= 0 && point.x < grid.width) {
                folderPoint = point
                break
            }
        }
        verify(folderPoint !== null, "视口内必须能找到一个文件夹")
        mouseDoubleClickSequence(grid, folderPoint.x, folderPoint.y)
        wait(50)
        compare(fakeBackend.openDirCalls, 1,
                "网格惯性期间的双击也必须一次打开文件夹")
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
        verify(grid.contentHeight > grid.height)
        compare(page.gridCellSlotWidth,
                Math.floor(Math.max(0, usableWidth - 1) / page.gridColumns))
        const occupiedWidth = (page.gridColumns - 1) * page.gridCellSlotWidth
                              + page.gridItemWidth
        verify(occupiedWidth <= usableWidth)
        const visualRightMargin = page.gridRightMargin
                                  + usableWidth - occupiedWidth
        const expectedRightMargin = page.gridSideMargin
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
        verify(page.gridItemWidth <= page.gridCellSlotWidth)
        const firstThumbnail = findChild(firstItem, "gridThumbnail")
        verify(firstThumbnail !== null)
        compare(firstThumbnail.radius,
                pageDensity.gridThumbnailRadius(page.gridItemWidth))
    }

    function test_grid_keeps_page_margin_when_content_fits() {
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

    function test_grid_geometry_stays_fixed_when_content_crosses_scroll_threshold() {
        fakeBackend.files = manyEntries(30)
        fakeBackend.contentView = "grid"
        page = pageComponent.createObject(parent, {
            width: 787,
            height: 500,
            visible: true
        })
        verify(page !== null)
        wait(80)

        const grid = findChild(page, "fileGrid")
        verify(grid !== null)
        verify(grid.contentHeight > grid.height)
        const cellWidth = grid.cellWidth
        const rightMargin = grid.rightMargin
        const itemWidth = page.gridItemWidth

        fakeBackend.files = manyEntries(5)
        tryCompare(grid, "count", 5)
        wait(40)
        verify(grid.contentHeight <= grid.height)
        compare(grid.cellWidth, cellWidth, "内容装得下时网格几何不能变")
        compare(grid.rightMargin, rightMargin, "滚动条出现不能改变右侧 margin")
        compare(page.gridItemWidth, itemWidth, "滚动条出现不能改变卡片宽度")

        fakeBackend.files = manyEntries(30)
        tryCompare(grid, "count", 30)
        wait(40)
        compare(grid.cellWidth, cellWidth, "内容再次溢出时网格几何也不能变")
        compare(grid.rightMargin, rightMargin)
        compare(page.gridItemWidth, itemWidth)

        const firstItem = grid.itemAtIndex(0)
        verify(firstItem !== null)
        let actualColumns = 0
        for (let i = 0; i < page.gridColumns; i++) {
            const item = grid.itemAtIndex(i)
            if (!item || Math.abs(item.y - firstItem.y) > 0.5)
                break
            actualColumns += 1
        }
        compare(actualColumns, page.gridColumns,
                "整行必须排满计算出的列数")
    }

    function test_search_placeholder_names_the_whole_drive() {
        searchBar = searchBarComponent.createObject(parent, { visible: true })
        verify(searchBar !== null)
        wait(20)

        const field = findChild(searchBar, "filesSearchField")
        verify(field !== null)
        compare(field.placeholderText, "搜索整个网盘")
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

    function test_header_actions_show_add_menu_and_keep_refresh_last() {
        fakeBackend.contentView = "list"
        fakeBackend.currentDirId = "folder-current"
        fakeBackend.breadcrumb = ["全部文件", "动画"]
        headerActions = headerActionsComponent.createObject(parent, { visible: true })
        verify(headerActions !== null)
        wait(30)

        const add = findChild(headerActions, "filesAddButton")
        const toggle = findChild(headerActions, "viewToggleButton")
        const refresh = findChild(headerActions, "filesRefreshButton")
        verify(add !== null && toggle !== null && refresh !== null)
        compare(headerActions.spacing, 8)
        compare(add.implicitHeight, 34)
        compare(toggle.implicitHeight, 34)
        compare(refresh.implicitHeight, 34)
        compare(add.text, qsTr("添加"))
        compare(add.variant, "primary")
        compare(add.trailingIconName, "chevron-down")
        compare(toggle.text, qsTr("列表"))
        compare(refresh.text, qsTr("刷新"))
        verify(add.x < toggle.x)
        verify(toggle.x < refresh.x, "刷新按钮应在最右侧")

        mouseClick(add)
        wait(30)

        const menu = findChild(headerActions, "filesAddMenu")
        const upload = findChild(menu, "filesAddUploadItem")
        const folderUpload = findChild(menu, "filesAddFolderUploadItem")
        const newFolder = findChild(menu, "filesAddNewFolderItem")
        const newTask = findChild(menu, "filesAddNewTaskItem")
        verify(menu !== null && menu.opened)
        verify(upload !== null && upload.enabled)
        verify(folderUpload !== null && folderUpload.enabled)
        verify(newFolder !== null && newFolder.enabled)
        verify(newTask !== null && newTask.enabled)
        compare(upload.text, qsTr("上传文件…"))
        compare(folderUpload.text, qsTr("上传文件夹…"))
        compare(newFolder.text, qsTr("新建文件夹…"))
        compare(newTask.text, qsTr("新建离线任务…"))
        verify(upload.y < folderUpload.y, "上传文件在上传文件夹之前")
        verify(folderUpload.y < newFolder.y, "上传文件夹在新建文件夹之前")

        mouseClick(upload)
        wait(20)
        compare(fakeBackend.uploadCalls, 1)
        compare(fakeBackend.uploadFolderId, "folder-current")
        compare(fakeBackend.uploadFolderName, "全部文件 / 动画")

        mouseClick(add)
        wait(20)
        mouseClick(findChild(menu, "filesAddFolderUploadItem"))
        wait(20)
        compare(fakeBackend.uploadFolderCalls, 1)
        compare(fakeBackend.uploadFolderFolderId, "folder-current")
        compare(fakeBackend.uploadFolderFolderName, "全部文件 / 动画")

        mouseClick(add)
        wait(20)
        mouseClick(newFolder)
        wait(20)
        compare(fakeBackend.newFolderCalls, 1)

        mouseClick(add)
        wait(20)
        mouseClick(findChild(headerActions, "filesAddNewTaskItem"))
        wait(20)
        compare(fakeBackend.newTaskDialogCalls, 1)
        compare(fakeBackend.newTaskDialogFolderId, "folder-current")
        compare(fakeBackend.newTaskDialogFolderPath, "全部文件 / 动画")
        compare(fakeBackend.newTaskDialogStartPath.length, 1)
        compare(fakeBackend.newTaskDialogStartPath[0].id, "")

        mouseClick(toggle)
        wait(20)
        compare(fakeBackend.contentView, "grid")

        mouseClick(refresh)
        wait(20)
        compare(fakeBackend.refreshCalls, 1)
    }

    function test_file_page_drops_files_into_the_current_directory() {
        fakeBackend.currentDirId = "folder-current"
        fakeBackend.breadcrumb = ["全部文件", "动画"]
        page = pageComponent.createObject(parent, { visible: true })
        verify(page !== null)
        wait(40)

        const dropRoot = findChild(page, "uploadDropRoot")
        verify(dropRoot !== null)
        compare(dropRoot.acceptsDrops, true)
        dropRoot.submitUrls(["file:///tmp/a%20b.mkv", "file:///tmp/c.mp4"])
        compare(fakeBackend.droppedUploadCalls, 1)
        compare(fakeBackend.droppedUploadFolderId, "folder-current")
        compare(fakeBackend.droppedUploadFolderName, "全部文件 / 动画")
        compare(fakeBackend.droppedUploadUrls.length, 2)

        fakeBackend.filesLoading = true
        wait(10)
        compare(dropRoot.acceptsDrops, false, "目录加载中不能继续接收拖放")
        fakeBackend.filesLoading = false
        fakeBackend.searchActive = true
        wait(10)
        compare(dropRoot.acceptsDrops, false, "搜索结果里不能把文件传到旧目录")
    }

    function test_add_menu_disables_folder_and_upload_while_searching() {
        fakeBackend.searchActive = true
        headerActions = headerActionsComponent.createObject(parent, { visible: true })
        verify(headerActions !== null)
        wait(30)

        const add = findChild(headerActions, "filesAddButton")
        verify(add !== null && add.enabled)
        mouseClick(add)
        wait(30)

        const upload = findChild(headerActions, "filesAddUploadItem")
        const folderUpload = findChild(headerActions, "filesAddFolderUploadItem")
        const newFolder = findChild(headerActions, "filesAddNewFolderItem")
        const newTask = findChild(headerActions, "filesAddNewTaskItem")
        verify(upload !== null && !upload.enabled)
        verify(folderUpload !== null && !folderUpload.enabled)
        verify(newFolder !== null && !newFolder.enabled)
        verify(newTask !== null && newTask.enabled)
    }

    function test_offline_task_completion_refreshes_only_visible_current_folder() {
        fakeBackend.currentDirId = "folder-current"
        fakeBackend.breadcrumb = ["全部文件", "当前目录"]
        page = pageComponent.createObject(parent, { visible: true })
        verify(page !== null)
        wait(40)

        fakeBackend.offlineTaskFolderChanged("folder-other")
        wait(20)
        compare(fakeBackend.backgroundRefreshCalls, 0,
                "别的目录完成离线任务不能刷新当前目录")

        fakeBackend.offlineTaskFolderChanged("folder-current")
        tryCompare(fakeBackend, "backgroundRefreshCalls", 1)

        fakeBackend.searchActive = true
        wait(20)
        fakeBackend.offlineTaskFolderChanged("folder-current")
        wait(20)
        compare(fakeBackend.backgroundRefreshCalls, 1,
                "搜索结果里不能刷新背后的旧目录")

        fakeBackend.searchActive = false
        tryCompare(fakeBackend, "backgroundRefreshCalls", 2)

        page.holdingSearchEntries = true
        wait(20)
        fakeBackend.offlineTaskFolderChanged("folder-current")
        wait(20)
        compare(fakeBackend.backgroundRefreshCalls, 2,
                "搜索结果过渡期间不能刷新背后的旧目录")
        page.holdingSearchEntries = false
        tryCompare(fakeBackend, "backgroundRefreshCalls", 3)

        page.visible = false
        wait(20)
        fakeBackend.offlineTaskFolderChanged("folder-current")
        wait(20)
        compare(fakeBackend.backgroundRefreshCalls, 3,
                "文件页不可见时不能立即刷新")

        page.visible = true
        tryCompare(fakeBackend, "backgroundRefreshCalls", 4)

        page.windowVisible = false
        wait(20)
        fakeBackend.offlineTaskFolderChanged("folder-current")
        wait(20)
        compare(fakeBackend.backgroundRefreshCalls, 4,
                "窗口收进托盘时不能立即刷新")

        page.windowVisible = true
        tryCompare(fakeBackend, "backgroundRefreshCalls", 5)
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

    // 卡片之间的缝隙也要能弹出空白菜单（排序入口在这里）。
    // GridView.indexAt 按 cell 命中，缝隙会被算进隔壁那张卡，
    // 所以容器另有 hitItemIndex 按卡片本体矩形判一次（docs/pitfalls.md 第 44 条）。
    function test_right_click_in_grid_seam_opens_blank_menu() {
        fakeBackend.files = manyEntries(30)
        fakeBackend.contentView = "grid"
        page = pageComponent.createObject(parent, {
            width: 716, height: 560, visible: true
        })
        verify(page !== null)
        wait(60)

        const grid = findChild(page, "fileGrid")
        verify(grid !== null)
        const container = grid.parent
        verify(container !== null)
        verify(page.gridColumns > 1, "要多列才测得出横向缝隙")

        const first = grid.itemAtIndex(0)
        const right = grid.itemAtIndex(1)
        const below = grid.itemAtIndex(page.gridColumns)
        verify(first !== null && right !== null && below !== null)
        const firstTop = first.mapToItem(container, 0, 0)
        const firstBottomRight = first.mapToItem(container, first.width, first.height)
        const rightLeft = right.mapToItem(container, 0, first.y)
        const belowTop = below.mapToItem(container, first.x, 0)

        // 缝隙取两张卡之间的中点，不靠固定间距数值
        const seamX = Qt.point((firstBottomRight.x + rightLeft.x) / 2,
                               firstTop.y + first.height / 2)
        const seamY = Qt.point(firstTop.x + first.width / 2,
                               (firstBottomRight.y + belowTop.y) / 2)

        compare(container.hitItemIndex(firstTop.x + 4, firstTop.y + 4), 0,
                "卡片本体算命中")
        compare(container.hitItemIndex(seamX.x, seamX.y), -1, "横向缝隙算空白")
        compare(container.hitItemIndex(seamY.x, seamY.y), -1, "纵向缝隙算空白")
        verify(grid.indexAt(seamX.x, seamX.y + grid.contentY) >= 0,
               "缝隙确实落在隔壁那张卡的 cell 里")

        let emittedCount = 0
        let emittedAt = Qt.point(-1, -1)
        const onBlankMenu = function (position) {
            emittedCount += 1
            emittedAt = position
        }
        container.blankContextMenuRequested.connect(onBlankMenu)

        mouseClick(container, seamX.x, seamX.y, Qt.RightButton)
        wait(40)
        compare(emittedCount, 1, "在横向缝隙右键要弹出空白菜单"
                + "（seam=" + seamX.x + "/" + seamX.y + ", card=" + firstTop.x + "/" + firstTop.y
                + ", cardSize=" + first.width + "/" + first.height
                + ", columns=" + page.gridColumns + "）")
        // 发出的坐标必须是网格自己的视口坐标：TapHandler 的 position 以内容区为原点
        // （带 leftMargin 偏移），页面再 mapToItem(null, …) 弹窗就会偏离光标。
        compare(Math.round(emittedAt.x), Math.round(seamX.x), "菜单跟随横向缝隙光标")
        compare(Math.round(emittedAt.y), Math.round(seamX.y), "菜单跟随横向缝隙光标")

        mouseClick(container, seamY.x, seamY.y, Qt.RightButton)
        wait(40)
        compare(emittedCount, 2, "在纵向缝隙右键要弹出空白菜单")
        compare(Math.round(emittedAt.x), Math.round(seamY.x), "菜单跟随纵向缝隙光标")
        compare(Math.round(emittedAt.y), Math.round(seamY.y), "菜单跟随纵向缝隙光标")

        // 落在卡片本体上的右键仍归条目菜单，不能抢走
        mouseClick(container, firstTop.x + first.width / 2,
                   firstTop.y + first.height / 2, Qt.RightButton)
        wait(40)
        compare(emittedCount, 2, "卡片本体右键不弹空白菜单")

        // 滚动后同样要对齐光标：TapHandler 的 position 还跟着 contentY 走
        grid.contentY = 200
        wait(40)
        const blankY = grid.height - 6
        mouseClick(container, 4, blankY, Qt.RightButton)
        wait(40)
        compare(emittedCount, 3, "滚动后左侧留白仍能弹出空白菜单")
        compare(Math.round(emittedAt.y), Math.round(blankY), "滚动后菜单位置不偏移")

        container.blankContextMenuRequested.disconnect(onBlankMenu)
    }

    // 兜底对照：最后一行下方的空白本来就能弹出菜单，
    // 用同一条链路证明右键确实能送到网格的 TapHandler。
    function test_right_click_below_grid_rows_opens_blank_menu() {
        fakeBackend.files = manyEntries(2)
        fakeBackend.contentView = "grid"
        page = pageComponent.createObject(parent, {
            width: 716, height: 560, visible: true
        })
        verify(page !== null)
        wait(60)

        const grid = findChild(page, "fileGrid")
        verify(grid !== null)
        const container = grid.parent
        verify(container !== null)

        const spy = Qt.createQmlObject(
            "import QtTest; SignalSpy { signalName: 'blankContextMenuRequested' }",
            container)
        spy.target = container
        mouseClick(container, 60, grid.height - 8, Qt.RightButton)
        wait(40)
        compare(spy.count, 1, "最后一行下方右键要弹出空白菜单")
        spy.destroy()
    }
}
