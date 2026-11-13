import QtQuick
import QtTest
import "../files"

// 文件页内部拖拽移动：
// - 拖动源按「按下时是否已选中」决定拖一条还是整个选择；
// - 搜索 / 回收站 / 临时行 / 写操作锁定时不启动；
// - 文件夹行与网格卡片作为落点时高亮，面包屑落点用并列的 breadcrumbIds；
// - 同目录 / 自己 / 自己的子目录不在 QML 里另做校验，交给 moveEntries 拒绝。
//
// 平台拖拽（QDrag 真正越过阈值）在 offscreen 下不可模拟，这里锁住的是
// 拖动数据、落点状态和后端调用参数；真实桌面上的拖拽手感仍需实机验收。
TestCase {
    name: "FileInternalDrag"
    when: windowShown
    width: 900
    height: 620

    QtObject {
        id: fakeBackend

        property var files: []
        property var searchResults: []
        property string authState: "signedIn"
        property string contentView: "list"
        property bool searchActive: false
        property bool searchSettled: true
        property bool searchTruncated: false
        property bool localIndexReady: true
        property string searchStatus: ""
        property string searchQuery: ""
        property bool filesLoading: false
        property bool filesRefreshing: false
        property bool filesStale: false
        property int filesRevision: 1
        property string currentDirId: ""
        property var breadcrumb: ["全部文件"]
        property var breadcrumbIds: [""]
        property var playMenu: []
        property var fileActionIds: []
        property bool trashClearing: false
        property bool shareActionRunning: false
        property string shareCreatedUrl: ""
        property string shareCreatedPassCode: ""
        property string shareCreatedExpiration: ""
        property var moveTargets: []
        property bool moveTargetsLoading: false
        property int moveTargetsRevision: 0
        property string moveTargetsError: ""
        property bool playLoading: false
        property string playLoadingId: ""
        property var playingIds: []

        property int moveCalls: 0
        property var lastMoveIds: []
        property string lastMoveTargetId: ""
        property string lastMoveTargetName: ""
        property bool moveAccepts: true

        signal changed()
        signal fileActionDone(string action, bool ok, string message, var ids)
        signal folderCreateStarted(string id)
        signal folderCreated(string id)
        signal offlineTaskFolderChanged(string folderId)
        signal shareActionDone(string action, bool ok, string message)
        signal thumbnailReady(string id, string path)

        function setContentView(view) {
            contentView = view === "grid" ? "grid" : "list"
        }
        function openDir(id) {}
        function openParent() {}
        function openCrumb(index) {}
        function exitSearch() {
            searchActive = false
        }
        function searchLocal(text) {}
        function searchAll() {}
        function searchOpenFolder(id) {}
        function searchRevealFolder(id) {}
        function requestPlay(id) {}
        function downloadEntry(id) {}
        function cancelPlayLoading() {}
        function thumbnailPath(id) { return "" }
        function requestThumbnail(id) {}
        function refreshFiles() {}
        function refreshFilesBackground() {}
        function ensureFilesFresh() {}
        function clearStatusMessage() {}
        function beginNewFolder() {}
        function chooseUploadFiles(folderId, folderName) {}
        function chooseUploadFolders(folderId, folderName) {}
        function uploadDroppedFiles(folderId, folderName, urls) {}
        function openNewTaskDialog(folderId, folderPath, startPath) {}
        function isDownloadPending(id) { return false }
        function hasPartialDownload(id) { return false }
        function setSorting(field, ascending) {}
        function currentPath() {
            return breadcrumb.length > 0
                    ? [{ id: currentDirId, name: breadcrumb[breadcrumb.length - 1] }]
                    : [{ id: "", name: "全部文件" }]
        }
        function loadMoveTargets(id) {}
        function cancelMoveTargets() {}
        function moveEntries(ids, targetId, targetName) {
            moveCalls += 1
            lastMoveIds = ids ? ids.slice() : []
            lastMoveTargetId = String(targetId || "")
            lastMoveTargetName = String(targetName || "")
            return moveAccepts
        }
    }

    Component {
        id: pageComponent

        FilesPage {
            width: 760
            height: 560
            backend: fakeBackend
        }
    }

    Component {
        id: bareCardComponent

        FileGridItem {
            width: 180
            height: 200
            backend: fakeBackend
        }
    }

    property var page: null

    function entry(index, kind) {
        return {
            id: "entry-" + index,
            name: kind === "folder" ? "文件夹 " + index : "文件 " + index + ".mkv",
            kind: kind,
            sizeText: kind === "folder" ? "—" : "1.2 GB",
            modifiedText: "2026-11-11 10:00",
            hasThumbnail: false,
            video: false,
            playable: false,
            image: false,
            auditFlag: false,
            starred: false,
            pending: false,
            inTrash: false
        }
    }

    function resetBackend() {
        fakeBackend.files = []
        fakeBackend.searchResults = []
        fakeBackend.authState = "signedIn"
        fakeBackend.contentView = "list"
        fakeBackend.searchActive = false
        fakeBackend.searchSettled = true
        fakeBackend.searchTruncated = false
        fakeBackend.searchStatus = ""
        fakeBackend.searchQuery = ""
        fakeBackend.filesLoading = false
        fakeBackend.filesRefreshing = false
        fakeBackend.filesStale = false
        fakeBackend.filesRevision += 1
        fakeBackend.currentDirId = ""
        fakeBackend.breadcrumb = ["全部文件"]
        fakeBackend.breadcrumbIds = [""]
        fakeBackend.fileActionIds = []
        fakeBackend.moveCalls = 0
        fakeBackend.lastMoveIds = []
        fakeBackend.lastMoveTargetId = ""
        fakeBackend.lastMoveTargetName = ""
        fakeBackend.moveAccepts = true
    }

    function loadFiles(entries) {
        fakeBackend.files = entries
        fakeBackend.filesRevision += 1
        fakeBackend.changed()
        tryCompare(page.activeFileView, "count", entries.length)
        wait(40)
    }

    function viewItem(index) {
        return page.activeFileView.itemAtIndex(index)
    }

    function init() {
        resetBackend()
        page = pageComponent.createObject(parent, { visible: true })
        verify(page !== null, "FilesPage 创建失败")
        wait(60)
    }

    function cleanup() {
        if (page) {
            page.visible = false
            wait(30)
            page.destroy()
            wait(30)
        }
        page = null
        resetBackend()
    }

    function test_unselected_row_drags_only_itself() {
        loadFiles([entry(0, "folder"), entry(1, "file"), entry(2, "file")])
        const row = viewItem(1)
        verify(row !== null)

        verify(page.internalDrag.begin(row.entry), "未选中的行也能开始拖动")
        compare(page.internalDrag.dragIds.length, 1)
        compare(page.internalDrag.dragIds[0], "entry-1")

        const payload = JSON.parse(page.internalDrag.payloadJson)
        compare(payload.sourceDir, "", "payload 带源目录")
        compare(payload.ids.length, 1)
        compare(payload.ids[0], "entry-1")
        compare(page.internalDrag.mimeData["application/x-kichi-file-ids"],
                page.internalDrag.payloadJson,
                "拖拽数据只放在自定义 MIME 里")
    }

    function test_selected_row_drags_the_whole_selection() {
        loadFiles([entry(0, "folder"), entry(1, "file"), entry(2, "folder")])

        const first = viewItem(0)
        const second = viewItem(2)
        verify(first !== null && second !== null)
        mouseClick(first, Math.round(first.width / 2), Math.round(first.height / 2))
        wait(20)
        mouseClick(second, Math.round(second.width / 2),
                   Math.round(second.height / 2), Qt.LeftButton,
                   Qt.ControlModifier)
        wait(20)

        verify(page.internalDrag.begin(viewItem(0).entry))
        compare(page.internalDrag.dragIds.length, 2)
        verify(page.internalDrag.dragIds.indexOf("entry-0") >= 0)
        verify(page.internalDrag.dragIds.indexOf("entry-2") >= 0)

        // 按在未选中的条目上时，仍然只拖这一条
        verify(page.internalDrag.begin(viewItem(1).entry))
        compare(page.internalDrag.dragIds.length, 1)
        compare(page.internalDrag.dragIds[0], "entry-1")
    }

    function test_press_prepares_drag_but_platform_drag_waits_for_threshold() {
        loadFiles([entry(0, "folder"), entry(1, "file")])
        const row = viewItem(1)
        verify(row !== null)

        mousePress(row, 24, 24)
        wait(10)
        compare(page.internalDrag.prepared, true, "按下即准备拖动数据")
        compare(page.internalDrag.dragging, false, "还没越过阈值，不改界面状态")

        mouseRelease(row, 24, 24)
        wait(20)
        compare(page.internalDrag.prepared, false)
        compare(page.internalDrag.dragging, false)
    }

    function test_search_disables_dragging() {
        loadFiles([entry(0, "folder"), entry(1, "file")])
        fakeBackend.searchActive = true
        fakeBackend.searchResults = [entry(5, "file")]
        fakeBackend.changed()
        tryCompare(page.activeFileView, "count", 1)
        wait(30)

        compare(page.internalDrag.enabled, false, "搜索结果里禁止拖拽")
        verify(!page.internalDrag.begin(fakeBackend.searchResults[0]))

        const row = viewItem(0)
        verify(row !== null)
        compare(row.dragPrepared, false, "搜索行不会准备拖拽数据")
    }

    function test_loading_and_trash_states_disable_dragging() {
        fakeBackend.filesLoading = true
        loadFiles([entry(0, "folder")])
        compare(page.internalDrag.enabled, false, "列表加载中不启动拖拽")

        fakeBackend.filesLoading = false
        const trashEntry = entry(1, "file")
        trashEntry.inTrash = true
        loadFiles([trashEntry])
        verify(!page.internalDrag.begin(viewItem(0).entry),
               "回收站条目不能作为拖动源")

        // 回收站 / 资料库复用的网格卡片没有协调器：落点与拖动源都是关闭的。
        const bare = bareCardComponent.createObject(parent, {
            index: 0,
            entry: entry(9, "folder")
        })
        verify(bare !== null)
        wait(20)
        compare(bare.internalDrag, null)
        compare(findChild(bare, "gridDropArea").enabled, false)
        bare.destroy()
    }

    function test_pending_and_locked_rows_do_not_start_a_drag() {
        const pendingEntry = entry(0, "folder")
        pendingEntry.pending = true
        loadFiles([pendingEntry])
        compare(page.internalDrag.begin(viewItem(0).entry), false,
                "还没拿到真实 id 的临时行不能拖")

        loadFiles([entry(0, "folder"), entry(1, "file")])
        fakeBackend.fileActionIds = ["entry-1"]
        wait(20)
        compare(page.internalDrag.begin(viewItem(1).entry), false,
                "同一目标正在写操作时不能拖")
    }

    function test_folder_row_is_the_only_list_drop_target() {
        loadFiles([entry(0, "folder"), entry(1, "file")])
        const folderRow = viewItem(0)
        const fileRow = viewItem(1)
        verify(folderRow !== null && fileRow !== null)

        compare(findChild(folderRow, "fileRowDropArea").enabled, true)
        compare(findChild(fileRow, "fileRowDropArea").enabled, false,
                "文件行不接受拖拽")

        compare(folderRow.dropHovered, false)
        verify(page.internalDrag.hoverFolder(folderRow.entry))
        wait(10)
        compare(folderRow.dropHovered, true, "文件夹行悬停高亮")
        compare(findChild(folderRow, "fileRowDropHighlight").visible, true)

        verify(!page.internalDrag.hoverFolder(fileRow.entry))
        wait(10)
        compare(folderRow.dropHovered, false, "换到无效落点要高亮消失")
        compare(fileRow.dropHovered, false)
    }

    function test_grid_card_drop_highlight_and_validity() {
        fakeBackend.contentView = "grid"
        loadFiles([entry(0, "folder"), entry(1, "file")])
        const folderCard = viewItem(0)
        const fileCard = viewItem(1)
        verify(folderCard !== null && fileCard !== null)

        compare(findChild(folderCard, "gridDropArea").enabled, true)
        compare(findChild(fileCard, "gridDropArea").enabled, false)
        verify(page.internalDrag.hoverFolder(folderCard.entry))
        wait(10)
        compare(folderCard.dropHovered, true)
        compare(findChild(folderCard, "gridDropHighlight").visible, true)
        compare(fileCard.dropHovered, false)
    }

    function test_drop_on_folder_calls_move_entries() {
        loadFiles([entry(0, "file"), entry(1, "folder"), entry(2, "file")])
        const fileRow = viewItem(0)
        const folderRow = viewItem(1)
        verify(page.internalDrag.begin(fileRow.entry))

        verify(page.internalDrag.dropOnFolder(folderRow.entry,
                                              page.internalDrag.payloadJson))
        compare(fakeBackend.moveCalls, 1)
        compare(fakeBackend.lastMoveTargetId, "entry-1")
        compare(fakeBackend.lastMoveTargetName, "文件夹 1")
        compare(fakeBackend.lastMoveIds.length, 1)
        compare(fakeBackend.lastMoveIds[0], "entry-0")

        // 文件行不是落点：不发请求
        compare(page.internalDrag.dropOnFolder(viewItem(2).entry,
                                               page.internalDrag.payloadJson), false)
        compare(fakeBackend.moveCalls, 1)
    }

    function test_breadcrumb_drop_uses_parallel_directory_ids() {
        fakeBackend.breadcrumb = ["全部文件", "动画", "第四季"]
        fakeBackend.breadcrumbIds = ["", "id-animation", "id-season-4"]
        fakeBackend.currentDirId = "id-season-4"
        loadFiles([entry(0, "file")])

        verify(page.internalDrag.begin(viewItem(0).entry))
        verify(page.internalDrag.dropOnCrumb(1, page.internalDrag.payloadJson))
        compare(fakeBackend.lastMoveTargetId, "id-animation",
                "落点 id 来自 breadcrumbIds，不是名字或当前目录")
        compare(fakeBackend.lastMoveTargetName, "动画")
        compare(fakeBackend.lastMoveIds[0], "entry-0")

        // 根目录是空串 id，同样走面包屑下标
        verify(page.internalDrag.begin(viewItem(0).entry))
        verify(page.internalDrag.dropOnCrumb(0, page.internalDrag.payloadJson))
        compare(fakeBackend.lastMoveTargetId, "")
        compare(fakeBackend.moveCalls, 2)
    }

    function test_same_directory_drop_is_left_to_backend_validation() {
        fakeBackend.breadcrumb = ["全部文件", "动画", "第四季"]
        fakeBackend.breadcrumbIds = ["", "id-animation", "id-season-4"]
        fakeBackend.currentDirId = "id-season-4"
        fakeBackend.moveAccepts = false
        loadFiles([entry(0, "file")])

        verify(page.internalDrag.begin(viewItem(0).entry))
        compare(page.internalDrag.dropOnCrumb(2, page.internalDrag.payloadJson),
                false, "拖到当前目录由 moveEntries 拒绝，QML 不另造一套校验")
        compare(fakeBackend.moveCalls, 1)
        compare(fakeBackend.lastMoveTargetId, fakeBackend.currentDirId,
                "同目录仍交给后端，由它给出错误提示")
    }

    function test_drag_paints_sources_and_shows_count_and_target() {
        loadFiles([entry(0, "folder"), entry(1, "file"), entry(2, "folder")])
        const fileRow = viewItem(1)
        const folderRow = viewItem(0)
        verify(fileRow !== null && folderRow !== null)

        const hint = findChild(page, "internalDragHint")
        const countLabel = findChild(page, "internalDragCount")
        const targetLabel = findChild(page, "internalDragTarget")
        verify(hint !== null && countLabel !== null && targetLabel !== null)
        compare(hint.visible, false, "没在拖动时不显示提示")

        verify(page.internalDrag.begin(fileRow.entry))
        page.internalDrag.dragStarted()
        wait(300)

        compare(page.internalDrag.dragging, true)
        compare(hint.visible, true, "越阈值后出现拖动提示")
        compare(countLabel.text, "拖动「文件 1.mkv」",
                "只拖一条时提示直接写文件名")
        compare(fileRow.dragSource, true, "被拖的行原地淡出")
        compare(folderRow.dragSource, false)
        verify(fileRow.opacity < 0.9, "拖动源的行透明度下降")

        // 悬停到文件夹：提示补上落点名字
        verify(page.internalDrag.hoverFolder(folderRow.entry))
        wait(20)
        compare(page.internalDrag.hoverName, "文件夹 0")
        compare(targetLabel.visible, true)
        compare(targetLabel.text, "→ 文件夹 0")

        // 松手 / 拖拽结束：提示与淡出一起收掉
        page.internalDrag.finishDrag()
        wait(300)
        compare(hint.visible, false)
        compare(fileRow.dragSource, false)
        verify(fileRow.opacity > 0.99, "拖动结束恢复不透明")
        compare(page.internalDrag.hoverName, "")

        // 多选拖动：提示只报数量，并且整组源条目一起淡出
        mouseClick(folderRow, Math.round(folderRow.width / 2),
                   Math.round(folderRow.height / 2))
        wait(20)
        mouseClick(fileRow, Math.round(fileRow.width / 2),
                   Math.round(fileRow.height / 2), Qt.LeftButton,
                   Qt.ControlModifier)
        wait(20)
        verify(page.internalDrag.begin(folderRow.entry))
        page.internalDrag.dragStarted()
        wait(300)
        compare(countLabel.text, "拖动 2 项")
        compare(folderRow.dragSource, true)
        compare(fileRow.dragSource, true)
        compare(viewItem(2).dragSource, false)
        page.internalDrag.finishDrag()
        wait(300)
    }

    function test_grid_card_also_fades_while_dragged() {
        fakeBackend.contentView = "grid"
        loadFiles([entry(0, "folder"), entry(1, "file")])
        const card = viewItem(1)
        verify(card !== null)

        verify(page.internalDrag.begin(card.entry))
        page.internalDrag.dragStarted()
        wait(300)
        compare(card.dragSource, true)
        verify(card.opacity < 0.9)

        page.internalDrag.finishDrag()
        wait(300)
        compare(card.dragSource, false)
        verify(card.opacity > 0.99)
    }

    function test_drag_preview_is_grabbed_for_the_pointer_ghost() {
        loadFiles([entry(0, "folder"), entry(1, "file"), entry(2, "folder")])
        const folderRow = viewItem(0)
        const fileRow = viewItem(1)
        verify(folderRow !== null && fileRow !== null)

        const preview = findChild(page, "fileDragPreview")
        verify(preview !== null, "页面里有一张共享拖影卡片")

        verify(page.internalDrag.begin(folderRow.entry))
        tryVerify(function () {
            return page.internalDrag.dragImageUrl.length > 0
        }, 2000, "按下后抓到拖影图片")
        verify(page.internalDrag.dragImageResult !== null,
               "拖拽期间持有 grab 结果，图片缓存不失效")
        // grab 结果放在 QQuickPixmapCache 里，URL 走 Qt 的 itemgrabber 方案
        verify(page.internalDrag.dragImageUrl.indexOf("itemgrabber:") === 0,
               "拖影走 grab 结果的图片 URL：" + page.internalDrag.dragImageUrl)

        compare(preview.itemCount, 1)
        compare(findChild(page, "dragPreviewName").text, "文件夹 0")
        compare(findChild(page, "dragPreviewCount").text, "1 项",
                "计数文字按实际条目数生成")
        compare(findChild(page, "dragPreviewStack") !== null, true)

        page.internalDrag.finishDrag()
        compare(page.internalDrag.dragImageUrl, "", "拖动结束释放拖影")
        compare(page.internalDrag.dragImageResult, null)

        // 多选：叠一层 + 「N 项」，拖的是什么一目了然
        mouseClick(folderRow, Math.round(folderRow.width / 2),
                   Math.round(folderRow.height / 2))
        wait(20)
        mouseClick(fileRow, Math.round(fileRow.width / 2),
                   Math.round(fileRow.height / 2), Qt.LeftButton,
                   Qt.ControlModifier)
        wait(20)
        verify(page.internalDrag.begin(fileRow.entry))
        tryVerify(function () {
            return page.internalDrag.dragImageUrl.length > 0
        }, 2000, "多选也重新抓图")
        compare(preview.itemCount, 2)
        compare(findChild(page, "dragPreviewCount").text, "2 项")
        compare(findChild(page, "dragPreviewName").text, "文件 1.mkv",
                "多选时拖影显示按下的那一条")
        page.internalDrag.finishDrag()
    }

    function test_invalid_payload_is_ignored() {
        loadFiles([entry(0, "file"), entry(1, "folder")])
        const folderRow = viewItem(1)

        compare(page.internalDrag.parsePayload("不是 JSON"), null)
        compare(page.internalDrag.parsePayload("{}"), null)
        compare(page.internalDrag.parsePayload("{\"sourceDir\":\"\",\"ids\":[]}"), null)
        compare(page.internalDrag.parsePayload(undefined), null)

        compare(page.internalDrag.dropOnFolder(folderRow.entry, "{}"), false)
        compare(fakeBackend.moveCalls, 0, "坏数据不触发移动")
    }
}
