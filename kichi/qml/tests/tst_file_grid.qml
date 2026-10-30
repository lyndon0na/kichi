import QtQuick
import QtTest
import ".."

// 文件页网格视图回归：
// - 网格使用 3:2 缩略图，尺寸由 FileListDensity 与委托共同决定；
// - 列表 / 网格切换沿用同一份文件和选择模型；
// - 两种视图各自记住滚动位置，切换回来不跳顶部。
TestCase {
    name: "FileGrid"
    when: windowShown
    width: 760
    height: 560

    QtObject {
        id: fakeBackend

        property var files: []
        property string fileView: "list"
        property bool searchActive: false
        property bool busy: false
        property bool filesStale: false
        property int filesRevision: 1
        property string currentDirId: ""
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

        function setFileView(view) {
            fileView = view === "grid" ? "grid" : "list"
        }

        function openDir(id) {}
        function openParent() {}
        function openCrumb(index) {}
        function refreshFiles() {}
        function exitSearch() {}
        function searchLocal(text) {}
        function searchAll() {}
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
            backend: fakeBackend
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
        id: viewMenuComponent

        FilesPageViewMenu {
            width: 84
            height: 30
            backend: fakeBackend
        }
    }

    property var density: null
    property var gridItem: null
    property var page: null
    property var viewMenu: null

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
            auditFlag: false
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
        if (viewMenu)
            viewMenu.destroy()
        page = null
        gridItem = null
        density = null
        viewMenu = null
        fakeBackend.files = []
        fakeBackend.fileView = "list"
        fakeBackend.searchActive = false
        fakeBackend.filesRevision = 1
        fakeBackend.thumbnailRequests = 0
    }

    function test_grid_item_uses_three_by_two_thumbnail_and_metadata() {
        density = densityComponent.createObject(parent)
        verify(density !== null)
        const width = 200
        const height = density.gridItemHeight(width)
        gridItem = gridItemComponent.createObject(parent, {
            width: width,
            height: height,
            contentPadding: density.gridCardPadding,
            cardSpacing: density.gridCardSpacing,
            textSpacing: density.gridTextSpacing,
            nameLineHeight: density.gridNameLineHeight,
            metaLineHeight: density.gridMetaLineHeight,
            textPixelSize: density.namePixelSize,
            metaPixelSize: density.metaPixelSize,
            iconPixelSize: density.gridIconPixelSize,
            videoBadgeSize: density.gridVideoBadgeSize,
            entry: entry(1, "folder")
        })
        verify(gridItem !== null)
        wait(30)

        const thumbnail = findChild(gridItem, "gridThumbnail")
        const fallback = findChild(gridItem, "thumbnailFallbackIcon")
        const name = findChild(gridItem, "gridName")
        const meta = findChild(gridItem, "gridMeta")
        verify(thumbnail !== null && fallback !== null && name !== null && meta !== null)
        compare(thumbnail.width, width - density.gridCardPadding * 2)
        compare(thumbnail.height, Math.ceil(thumbnail.width * 2 / 3))
        compare(fallback.visible, true)
        compare(name.text, "目录 1")
        verify(meta.text.indexOf("文件夹") === 0)
        compare(fakeBackend.thumbnailRequests, 0, "文件夹不应请求缩略图")
    }

    function test_selected_grid_item_has_accent_ring() {
        density = densityComponent.createObject(parent)
        gridItem = gridItemComponent.createObject(parent, {
            width: 180,
            height: density.gridItemHeight(180),
            contentPadding: density.gridCardPadding,
            cardSpacing: density.gridCardSpacing,
            textSpacing: density.gridTextSpacing,
            nameLineHeight: density.gridNameLineHeight,
            metaLineHeight: density.gridMetaLineHeight,
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
        fakeBackend.fileView = "list"
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

        page.setFileView("grid")
        wait(50)
        compare(fakeBackend.fileView, "grid")
        compare(list.visible, false)
        compare(grid.visible, true)
        grid.contentY = 96
        wait(30)

        page.setFileView("list")
        wait(50)
        compare(fakeBackend.fileView, "list")
        compare(list.visible, true)
        compare(grid.visible, false)
        tryCompare(list, "contentY", 48)
    }

    function test_grid_layout_fills_width_without_a_phantom_column() {
        fakeBackend.files = manyEntries(30)
        fakeBackend.fileView = "grid"
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
        compare(page.gridCellSlotWidth, Math.floor(usableWidth / page.gridColumns))
        const pageDensity = findChild(page, "fileListDensity")
        verify(pageDensity !== null)
        verify(page.gridItemWidth + pageDensity.gridColumnGap <= page.gridCellSlotWidth)
    }

    function test_header_menu_requests_grid_and_disables_during_search() {
        fakeBackend.fileView = "list"
        viewMenu = viewMenuComponent.createObject(parent, { visible: true })
        verify(viewMenu !== null)
        wait(30)

        const button = findChild(viewMenu, "viewMenuButton")
        verify(button !== null)
        compare(button.text, qsTr("列表"))
        compare(button.enabled, true)

        const spy = Qt.createQmlObject(
            "import QtTest; SignalSpy { signalName: 'viewRequested' }",
            viewMenu)
        spy.target = viewMenu
        mouseClick(button)
        wait(30)
        const gridOption = findChild(viewMenu, "viewGridOption")
        verify(gridOption !== null)
        mouseClick(gridOption)
        wait(30)
        compare(spy.count, 1)
        compare(spy.signalArguments[0][0], "grid")

        fakeBackend.fileView = "grid"
        fakeBackend.searchActive = true
        compare(button.text, qsTr("网格"))
        compare(button.enabled, false)
        spy.destroy()
    }
}
