import QtQuick
import QtQuick.Controls
import QtTest
import ".."

// 离线任务页列表差异算法的回归测试（2026-10-28）。
//
// 为什么要有：后台每 10 秒推一次新任务列表。以前直接把 `backend.tasks`
// （每次都是新数组）赋给 `ListView.model`，ListView 整表重灌，用户滚到哪都会
// 被甩回顶部（`docs/known-issues.md` §4.1）。现在换 ListModel 就地做差异，
// 但差异算法本身容易写出 `move` 越界 / 游标不前进（传输页 2026-10-27 踩过），
// 因此把真实组件加载起来、用假后端驱动快照，直接断言行的顺序与滚动位置。
//
// 运行：
//   QT_QPA_PLATFORM=offscreen /usr/lib64/qt6/bin/qmltestrunner \
//       -input kichi/qml/tests/tst_tasks_page.qml

TestCase {
    name: "TasksPageRows"
    when: windowShown
    width: 800
    height: 600

    // 替代 Kichi 桥接对象：只提供 TasksPage 用到的属性 / 信号 / 方法。
    QtObject {
        id: backend

        property var tasks: []
        property bool tasksLoading: false
        property bool tasksClearing: false
        property bool playLoading: false
        property string playLoadingId: ""
        property var playingIds: []
        property string taskFolderLoadingId: ""
        property var moveTargets: []
        property int moveTargetsRevision: 0
        property bool moveTargetsLoading: false
        property string moveTargetsError: ""

        signal changed()
        signal thumbnailReady(string id, string path)
        signal torrentParsed(string name, string magnet, string detail)
        signal torrentParseFailed(string message)

        property int submitted: 0
        property string submittedText: ""
        property string submittedFolderId: ""
        property bool submittedToFolder: false
        property string deletedId: ""
        property int refreshCalls: 0
        property int thumbnailRequests: 0
        property int cancelPlayLoadingCalls: 0
        property int cancelMoveTargetsCalls: 0
        property int clearFinishedCalls: 0
        property int torrentPickerCalls: 0
        property int torrentPreviewCalls: 0
        property var torrentPreviewUrls: []

        function addTask(url) {
            submitted += 1
            submittedText = url
            submittedToFolder = false
        }
        function addTaskToFolder(url, folderId) {
            submitted += 1
            submittedText = url
            submittedFolderId = String(folderId)
            submittedToFolder = true
        }
        function deleteTask(id) {
            deletedId = id
        }
        function refreshTasks() {
            refreshCalls += 1
        }
        function clearFinishedTasks() {
            clearFinishedCalls += 1
        }
        function chooseTorrentFiles() {
            torrentPickerCalls += 1
        }
        function previewTorrentFiles(urls) {
            torrentPreviewCalls += 1
            torrentPreviewUrls = urls.slice()
        }
        function thumbnailPath(id) {
            return ""
        }
        function requestThumbnail(id) {
            thumbnailRequests += 1
        }
        function cancelPlayLoading() {
            cancelPlayLoadingCalls += 1
        }
        function currentPath() {
            return [{ id: "", name: "全部文件" }]
        }
        function loadMoveTargets(parentId) {
            if (parentId === "")
                moveTargets = [{ id: "A", name: "动画" }]
            else if (parentId === "A")
                moveTargets = [{ id: "B", name: "2026" }]
            else
                moveTargets = []
            moveTargetsRevision += 1
            moveTargetsLoading = false
            moveTargetsError = ""
            changed()
        }
        function cancelMoveTargets() {
            cancelMoveTargetsCalls += 1
        }
    }

    TasksPage {
        id: page
        backend: backend
        width: 800
        height: 600

        TaskHeaderControls {
            id: taskHeader
            y: 0
            width: 700
            height: 34
            target: page
        }
    }

    Component {
        id: interactivePageComponent

        TasksPage {}
    }

    // 任务列表（TasksPage 里只有一个 ListView，按 objectName 取更稳）
    property var list: findChild(page, "tasksList")
    property var model: list ? list.model : null
    property var loadingPage: null

    function init() {
        page.active = false
        list.cancelFlick()
        list.contentY = 0
        backend.tasksLoading = false
        backend.tasksClearing = false
        backend.refreshCalls = 0
        backend.submitted = 0
        backend.submittedText = ""
        backend.submittedFolderId = ""
        backend.submittedToFolder = false
        backend.playLoading = false
        backend.playLoadingId = ""
        backend.playingIds = []
        backend.taskFolderLoadingId = ""
        backend.moveTargets = []
        backend.moveTargetsRevision = 0
        backend.moveTargetsLoading = false
        backend.moveTargetsError = ""
        backend.thumbnailRequests = 0
        backend.cancelPlayLoadingCalls = 0
        backend.cancelMoveTargetsCalls = 0
        backend.clearFinishedCalls = 0
        backend.torrentPickerCalls = 0
        backend.torrentPreviewCalls = 0
        backend.torrentPreviewUrls = []
        page.width = 800
        page.active = true
        backend.refreshCalls = 0
        push([])
        page.pendingScrollRestore = false
        page.savedContentY = 0
    }

    function cleanup() {
        if (loadingPage)
            loadingPage.destroy()
        loadingPage = null
    }

    /// 造一条与 Rust `task_map` 同形状的任务
    function task(id, phase, progress) {
        var settled = phase === "completed" || phase === "error"
                || phase === "deleted"
        return {
            id: id,
            name: id + ".mkv",
            kind: "",
            phase: phase,
            phaseText: phase,
            progress: progress,
            progressText: progress + "%",
            sizeText: "1 GB",
            createdText: "2026-10-28 12:00",
            detail: "",
            settled: settled,
            fileId: id + "-file",
            hasThumbnail: false,
            video: false,
            playable: false
        }
    }

    function push(tasks) {
        backend.tasks = tasks
        backend.changed()
    }

    function test_empty_loads_use_skeleton_each_time() {
        backend.tasks = []
        backend.tasksLoading = true
        loadingPage = interactivePageComponent.createObject(parent, {
            width: 800,
            height: 600,
            visible: true,
            active: false,
            backend: backend
        })
        verify(loadingPage !== null)

        loadingPage.active = true
        wait(30)
        const skeleton = findChild(loadingPage, "pageLoadingSkeleton")
        const loadingState = findChild(loadingPage, "tasksLoadingState")
        verify(skeleton !== null && loadingState !== null)
        compare(skeleton.visible, true, "空内容读取显示骨架")
        compare(loadingState.visible, false, "空内容读取不叠原来的转圈")

        backend.tasksLoading = false
        push([task("first", "completed", 100)])
        wait(30)
        compare(skeleton.visible, false, "数据到达后收起骨架")

        backend.tasksLoading = true
        backend.changed()
        wait(30)
        compare(skeleton.visible, false, "有旧内容时刷新保留内容，不闪骨架")

        push([])
        wait(30)
        compare(skeleton.visible, true, "再次空内容读取仍显示骨架")
        compare(loadingState.visible, false, "再次空内容读取不叠原来的转圈")
    }

    function visibleRow(id) {
        for (var i = 0; i < list.count; i++) {
            var row = list.itemAtIndex(i)
            if (row && row.key === id)
                return row
        }
        return null
    }

    function test_rows_follow_backend_order() {
        push([task("t1", "running", 10), task("t2", "completed", 100)])
        compare(model.count, 2)
        compare(model.get(0).key, "t1")
        compare(model.get(1).key, "t2")
    }

    function test_scroll_stops_at_bounds() {
        compare(list.boundsBehavior, Flickable.StopAtBounds)
    }

    function test_table_layout_uses_compact_rows() {
        push([task("t1", "running", 10)])

        var header = findChild(page, "tasksTableHeader")
        verify(header)
        compare(header.implicitHeight, 34)
        compare(list.spacing, 0)
        compare(findChild(page, "tasksTableLayout").spacing, 4)

        var row = null
        tryVerify(function () {
            row = visibleRow("t1")
            return row !== null
        })
        compare(row.implicitHeight, 48)
        compare(row.name, "t1.mkv")
        compare(row.phaseText, "running")
        compare(row.thumbnailColumnWidth, 36)
        compare(row.sizeColumnWidth, 84)
        compare(row.createdColumnWidth, 120)
        compare(row.statusColumnWidth, 100)
        compare(row.progressColumnWidth, 150)

        var thumbnail = findChild(row, "taskThumbnail")
        verify(thumbnail)
        var typeIcon = findChild(thumbnail, "thumbnailFallbackIcon")
        verify(typeIcon)
        compare(typeIcon.iconName, "video")

        var nameText = findChild(row, "taskNameText")
        var sizeText = findChild(row, "taskSizeText")
        var createdText = findChild(row, "taskCreatedText")
        verify(nameText)
        verify(sizeText)
        verify(createdText)
        compare(nameText.font.pixelSize, 13)
        compare(sizeText.text, "1 GB")
        compare(createdText.text, "2026-10-28 12:00")

        var sizeHeader = findChild(header, "tasksSizeHeader")
        var createdHeader = findChild(header, "tasksCreatedHeader")
        var statusHeader = findChild(header, "tasksStatusHeader")
        verify(sizeHeader)
        verify(createdHeader)
        verify(statusHeader)
        compare(sizeHeader.text, "大小")
        compare(createdHeader.text, "创建时间")
        verify(findChild(header, "tasksActionHeader") === null,
               "操作列已移除，删除只放在右键菜单")
        var progressHeader = findChild(header, "tasksProgressHeader")
        verify(progressHeader)
        compare(progressHeader.horizontalAlignment, Text.AlignHCenter)

        var statusX = statusHeader.mapToItem(header, 0, 0).x
        var progressX = progressHeader.mapToItem(header, 0, 0).x
        var sizeX = sizeHeader.mapToItem(header, 0, 0).x
        var createdX = createdHeader.mapToItem(header, 0, 0).x
        verify(statusX < progressX, "表头状态列应在进度列之前")
        verify(progressX < sizeX, "表头大小列应在进度列之后")
        verify(sizeX < createdX, "表头创建时间列应在大小列之后")

        var statusSlot = findChild(row, "taskStatusSlot")
        var progressSlot = findChild(row, "taskProgressSlot")
        verify(statusSlot)
        verify(progressSlot)
        var rowStatusX = statusSlot.mapToItem(row, 0, 0).x
        var rowProgressX = progressSlot.mapToItem(row, 0, 0).x
        var rowSizeX = sizeText.mapToItem(row, 0, 0).x
        var rowCreatedX = createdText.mapToItem(row, 0, 0).x
        verify(rowStatusX < rowProgressX, "行状态列应在进度列之前")
        verify(rowProgressX < rowSizeX, "行大小列应在进度列之后")
        verify(rowSizeX < rowCreatedX, "行创建时间列应在大小列之后")
    }

    function test_api_folder_kind_wins_over_name_fallback() {
        var folder = task("folder", "completed", 100)
        folder.name = "电影合集"
        folder.kind = "folder"
        push([folder])

        var row = null
        tryVerify(function () {
            row = visibleRow("folder")
            return row !== null
        })
        var thumbnail = findChild(row, "taskThumbnail")
        verify(thumbnail)
        var typeIcon = findChild(thumbnail, "thumbnailFallbackIcon")
        verify(typeIcon)
        compare(typeIcon.iconName, "folder")
    }

    function test_thumbnail_uses_completed_resource_file_id() {
        var done = task("thumb", "completed", 100)
        done.fileId = "file-thumb"
        done.hasThumbnail = true
        done.video = true
        done.playable = true
        push([done])

        tryVerify(function () {
            return backend.thumbnailRequests === 1
        })
        var row = null
        tryVerify(function () {
            row = visibleRow("thumb")
            return row !== null
        })
        var thumbnail = findChild(row, "taskThumbnail")
        verify(thumbnail)
        compare(thumbnail.entry.id, "file-thumb")
        tryVerify(function () {
            return thumbnail.showVideoBadge
        }, 1000,
                  "可播放视频即使缩略图未返回也保留播放角标；row.video="
                  + row.video + " row.playable=" + row.playable
                  + " entry.video=" + thumbnail.entry.video
                  + " entry.playable=" + thumbnail.entry.playable
                  + " loading=" + thumbnail.loading
                  + " playing=" + thumbnail.playing
                  + " showVideoBadge=" + thumbnail.showVideoBadge
                  + " row.visible=" + row.visible)
    }

    // 加载中的产物不在当前离线任务列表里（换标签页 / 离开任务页）时不再内联显示，
    // 底部提示条据此接管取消入口。
    function test_inline_play_loading_follows_visible_tasks() {
        var done = task("inline", "completed", 100)
        done.fileId = "inline-file"
        push([done])
        backend.playLoading = true
        backend.playLoadingId = "inline-file"
        tryCompare(page, "showsInlinePlayLoading", true)

        backend.playLoadingId = "other-file"
        tryCompare(page, "showsInlinePlayLoading", false)

        backend.playLoadingId = "inline-file"
        tryCompare(page, "showsInlinePlayLoading", true)
        page.active = false
        tryCompare(page, "showsInlinePlayLoading", false)
    }

    function test_loading_and_playing_follow_the_resource_id() {
        var done = task("state", "completed", 100)
        done.fileId = "file-state"
        done.video = true
        done.playable = true
        push([done])

        var row = null
        tryVerify(function () {
            row = visibleRow("state")
            return row !== null
        })
        backend.playLoading = true
        backend.playLoadingId = "file-state"
        tryVerify(function () {
            return row.playLoading === true
        })
        var overlay = findChild(row, "taskVideoLoading")
        var thumbnail = findChild(row, "taskThumbnail")
        verify(overlay)
        compare(overlay.parent, row)
        compare(overlay.x, row.contentPadding)
        compare(thumbnail.loading, true)
        compare(thumbnail.showVideoBadge, false)

        backend.playLoading = false
        backend.playingIds = ["file-state"]
        tryVerify(function () {
            return thumbnail.playing === true
        })
        compare(thumbnail.showVideoBadge, false)
    }

    function test_folder_opening_leaves_row_unchanged_and_blocks_repeat_activation() {
        var folder = task("opening-folder", "completed", 100)
        folder.kind = "folder"
        folder.fileId = "opening-folder-id"
        push([folder])

        var row = null
        tryVerify(function () {
            row = visibleRow("opening-folder")
            return row !== null
        })
        var spy = Qt.createQmlObject(
                    "import QtTest; SignalSpy { signalName: 'openFolderRequested' }", page)
        spy.target = page

        backend.taskFolderLoadingId = "opening-folder-id"
        tryVerify(function () {
            return row.folderLoading === true
        })
        compare(row.canActivate, false)
        compare(row.rowHighlighted, false)

        var thumbnail = findChild(row, "taskThumbnail")
        var statusBadge = findChild(row, "taskStatusBadge")
        verify(thumbnail)
        verify(statusBadge)
        compare(findChild(row, "taskFolderLoadingState"), null,
                "任务行不再显示跳转加载态")
        compare(thumbnail.loading, false, "文件夹跳转不能复用播放加载态")
        compare(thumbnail.opacity, 1, "缩略图不能因跳转而压暗")
        compare(statusBadge.opacity, 1)

        row.activate(Qt.point(30, 20))
        wait(20)
        compare(spy.count, 0, "加载期间重复双击不能再次发起目录跳转")

        backend.taskFolderLoadingId = ""
        tryVerify(function () {
            return row.folderLoading === false
        })
        compare(row.canActivate, true)
        compare(thumbnail.loading, false)
        compare(thumbnail.opacity, 1)
        compare(statusBadge.opacity, 1)
        row.activate(Qt.point(30, 20))
        wait(20)
        compare(spy.count, 1)

        spy.destroy()
    }

    function test_double_click_opens_folder_or_plays_completed_resource() {
        var folderSpy = Qt.createQmlObject(
                    "import QtTest; SignalSpy { signalName: 'openFolderRequested' }", page)
        folderSpy.target = page
        var playSpy = Qt.createQmlObject(
                    "import QtTest; SignalSpy { signalName: 'playRequested' }", page)
        playSpy.target = page

        var folder = task("open-folder", "completed", 100)
        folder.kind = "folder"
        folder.fileId = "folder-id"
        push([folder])
        var row = null
        tryVerify(function () {
            row = visibleRow("open-folder")
            return row !== null
        })
        row.activate(Qt.point(30, 20))
        wait(20)
        compare(folderSpy.count, 1)
        compare(folderSpy.signalArguments[0][0], "folder-id")

        var video = task("open-video", "completed", 100)
        video.name = "电影.mkv"
        video.fileId = "video-id"
        video.video = true
        video.playable = true
        push([video])
        tryVerify(function () {
            row = visibleRow("open-video")
            return row !== null
        })
        row.activate(Qt.point(30, 20))
        wait(20)
        compare(playSpy.count, 1)
        compare(playSpy.signalArguments[0][0], "video-id")

        folderSpy.destroy()
        playSpy.destroy()
    }

    function test_double_click_after_flick_opens_completed_folder_once() {
        var interactivePage = interactivePageComponent.createObject(parent, {
            width: 800,
            height: 600,
            visible: true,
            active: true,
            backend: backend
        })
        verify(interactivePage !== null)
        var testList = findChild(interactivePage, "tasksList")
        verify(testList !== null)
        var folderSpy = Qt.createQmlObject(
                    "import QtTest; SignalSpy { signalName: 'openFolderRequested' }",
                    interactivePage)
        folderSpy.target = interactivePage

        var folders = []
        for (var i = 0; i < 40; i++) {
            var folder = task("flick-folder-" + i, "completed", 100)
            folder.name = "文件夹 " + i
            folder.kind = "folder"
            folder.fileId = "flick-folder-id-" + i
            folders.push(folder)
        }
        backend.tasks = folders
        backend.changed()
        tryVerify(function () {
            return testList.count === folders.length
        })
        tryVerify(function () {
            return testList.height > 0
                    && testList.contentHeight > testList.height
        }, 2000, "列表应有可滚动高度："
                  + testList.height + " / " + testList.contentHeight)

        testList.flick(0, -600)
        wait(30)
        verify(testList.moving, "测试必须覆盖惯性滚动期间")
        const x = 120
        const y = testList.height / 2
        const index = testList.indexAt(x, y + testList.contentY)
        verify(index >= 0)
        const row = testList.itemAtIndex(index)
        verify(row !== null && row.canOpenFolder)

        mouseDoubleClickSequence(testList, x, y)
        wait(50)
        compare(folderSpy.count, 1, "惯性后的双击必须一次打开文件夹")
        compare(folderSpy.signalArguments[0][0], row.fileId)

        folderSpy.destroy()
        testList.cancelFlick()
        interactivePage.destroy()
    }

    function test_failed_resource_does_not_activate() {
        var failed = task("failed", "error", 0)
        failed.kind = "folder"
        failed.fileId = "failed-folder"
        failed.name = "failed.mkv"
        failed.playable = true
        failed.video = true
        push([failed])

        var row = null
        tryVerify(function () {
            row = visibleRow("failed")
            return row !== null
        })
        compare(row.canOpenFolder, false)
        compare(row.canPlay, false)
        compare(row.canActivate, false)
    }

    function test_play_badge_only_appears_for_completed_tasks() {
        var phases = ["pending", "running", "paused", "error", "deleted", "unknown",
                      "completed"]
        var tasks = []
        for (var i = 0; i < phases.length; i++) {
            var item = task("badge-" + phases[i], phases[i], 50)
            item.kind = "file"
            item.fileId = "badge-file-" + phases[i]
            item.video = true
            item.playable = true
            tasks.push(item)
        }
        push(tasks)

        for (var j = 0; j < phases.length; j++) {
            var row = null
            tryVerify(function () {
                row = visibleRow("badge-" + phases[j])
                return row !== null
            })
            var thumbnail = findChild(row, "taskThumbnail")
            verify(thumbnail)
            compare(thumbnail.showVideoBadge, phases[j] === "completed",
                    phases[j] + " status play badge")
        }
    }

    function test_status_badges_cover_every_task_state() {
        var cases = [
            ["pending", "排队中", "accent", false],
            ["running", "下载中", "accent", false],
            ["paused", "已暂停", "neutral", false],
            ["completed", "已完成", "success", true],
            ["error", "失败", "danger", true],
            ["deleted", "产物已删除", "warning", true],
            ["unknown", "状态未知", "warning", false]
        ]
        var tasks = []
        for (var i = 0; i < cases.length; i++) {
            var item = task("status-" + cases[i][0], cases[i][0], 0)
            item.phaseText = cases[i][1]
            if (cases[i][0] === "deleted")
                item.detail = "File deleted"
            tasks.push(item)
        }
        push(tasks)

        for (var j = 0; j < cases.length; j++) {
            var id = "status-" + cases[j][0]
            var row = null
            tryVerify(function () {
                row = visibleRow(id)
                return row !== null
            })
            compare(row.tone, cases[j][2], id + " tone")
            compare(row.isActive, !cases[j][3], id + " active state")
            var badge = findChild(row, "taskStatusBadge")
            verify(badge)
            compare(badge.text, cases[j][1], id + " label")
            compare(badge.tone, cases[j][2], id + " badge tone")
        }
    }

    function test_deleted_output_keeps_official_reason_in_tooltip() {
        var deleted = task("deleted-detail", "deleted", 100)
        deleted.phaseText = "产物已删除"
        deleted.detail = "File deleted"
        deleted.kind = "file"
        deleted.fileId = "gone-file"
        deleted.playable = true
        push([deleted])

        var row = null
        tryVerify(function () {
            row = visibleRow("deleted-detail")
            return row !== null
        })
        compare(row.showProgress, false)
        compare(row.canActivate, false)
        compare(row.statusHasDetail, true)
        verify(row.statusDetail.indexOf("File deleted") >= 0)
        verify(row.statusDetail.indexOf("已从云盘删除") >= 0)
    }

    function test_delete_is_only_available_from_right_click_menu() {
        push([task("delete-me", "completed", 100)])
        var row = null
        tryVerify(function () {
            row = visibleRow("delete-me")
            return row !== null
        })
        row.contextMenu(Qt.point(40, 20))
        wait(40)

        var menu = findChild(page, "taskContextMenu")
        verify(menu)
        verify(menu.opened)
        var item = findChild(menu, "taskDeleteItem")
        verify(item)
        mouseClick(item)
        wait(40)

        var dialog = findChild(page, "deleteTaskDialog")
        verify(dialog)
        verify(dialog.opened)
        compare(page.pendingDeleteId, "delete-me")
        var ok = findChild(dialog, "dialogFooterOk")
        verify(ok !== null, "删除弹窗有「删除」按钮")
        compare(ok.text, "删除")
        compare(ok.variant, "danger")
        mouseClick(ok)
        wait(40)
        compare(backend.deletedId, "delete-me")
    }

    function test_context_menu_matches_double_click_open_actions() {
        var folderSpy = Qt.createQmlObject(
                    "import QtTest; SignalSpy { signalName: 'openFolderRequested' }", page)
        folderSpy.target = page
        var playSpy = Qt.createQmlObject(
                    "import QtTest; SignalSpy { signalName: 'playRequested' }", page)
        playSpy.target = page

        var folder = task("context-folder", "completed", 100)
        folder.kind = "folder"
        folder.fileId = "context-folder-id"
        push([folder])
        var row = null
        tryVerify(function () {
            row = visibleRow("context-folder")
            return row !== null
        })
        row.contextMenu(Qt.point(40, 20))
        wait(40)

        var openItem = findChild(page, "taskOpenFolderItem")
        var playItem = findChild(page, "taskPlayItem")
        verify(openItem.visible)
        compare(openItem.text, "打开文件夹")
        compare(playItem.visible, false)
        mouseClick(openItem)
        wait(40)
        compare(folderSpy.count, 1)
        compare(folderSpy.signalArguments[0][0], "context-folder-id")

        var video = task("context-video", "completed", 100)
        video.kind = "file"
        video.name = "电影.mkv"
        video.fileId = "context-video-id"
        video.video = true
        video.playable = true
        push([video])
        tryVerify(function () {
            row = visibleRow("context-video")
            return row !== null
        })
        row.contextMenu(Qt.point(40, 20))
        wait(40)

        openItem = findChild(page, "taskOpenFolderItem")
        playItem = findChild(page, "taskPlayItem")
        var revealItem = findChild(page, "taskRevealLocationItem")
        compare(openItem.visible, false)
        verify(playItem.visible)
        verify(revealItem.visible)
        compare(playItem.text, "播放")
        verify(playItem.y < revealItem.y, "媒体文件的播放入口应排在打开所在位置之前")
        mouseClick(playItem)
        wait(40)
        compare(playSpy.count, 1)
        compare(playSpy.signalArguments[0][0], "context-video-id")

        folderSpy.destroy()
        playSpy.destroy()
    }

    function test_context_menu_reveals_completed_file_and_folder_locations() {
        var revealSpy = Qt.createQmlObject(
                    "import QtTest; SignalSpy { signalName: 'revealLocationRequested' }", page)
        revealSpy.target = page

        var folder = task("reveal-folder", "completed", 100)
        folder.kind = "folder"
        folder.fileId = "reveal-folder-id"
        push([folder])
        var row = null
        tryVerify(function () {
            row = visibleRow("reveal-folder")
            return row !== null
        })
        row.contextMenu(Qt.point(40, 20))
        wait(40)

        var revealItem = findChild(page, "taskRevealLocationItem")
        verify(revealItem)
        verify(revealItem.visible)
        compare(revealItem.text, "打开目录所在位置")
        mouseClick(revealItem)
        wait(40)
        compare(revealSpy.count, 1)
        compare(revealSpy.signalArguments[0][0], "reveal-folder-id")

        var file = task("reveal-file", "completed", 100)
        file.kind = "file"
        file.fileId = "reveal-file-id"
        push([file])
        tryVerify(function () {
            row = visibleRow("reveal-file")
            return row !== null
        })
        row.contextMenu(Qt.point(40, 20))
        wait(40)

        revealItem = findChild(page, "taskRevealLocationItem")
        verify(revealItem)
        verify(revealItem.visible)
        compare(revealItem.text, "打开文件所在位置")
        mouseClick(revealItem)
        wait(40)
        compare(revealSpy.count, 2)
        compare(revealSpy.signalArguments[1][0], "reveal-file-id")

        revealSpy.destroy()
    }

    function test_size_and_time_columns_hide_before_the_name_is_crushed() {
        push([task("responsive", "completed", 100)])
        var header = findChild(page, "tasksTableHeader")
        verify(header)
        compare(header.showSize, true)
        compare(header.showCreated, true)

        page.width = 650
        tryVerify(function () {
            return header.showCreated === false
        })
        compare(header.showSize, true)

        page.width = 520
        tryVerify(function () {
            return header.showSize === false
        })
    }

    function test_header_controls_show_actions_without_status() {
        compare(findChild(taskHeader, "tasksHeaderStatus"), null,
                "离线任务标题旁不再显示任务数量")

        push([
            task("active", "running", 42),
            task("done", "completed", 100)
        ])
        verify(findChild(taskHeader, "newTaskButton"))
        verify(findChild(taskHeader, "tasksRefreshButton"))
    }

    function test_new_task_dialog_submits_multiline_text() {
        page.openNewTaskDialog()

        var dialog = findChild(page, "newTaskDialog")
        verify(dialog)
        tryVerify(function () {
            return dialog.opened
        })

        var headerBottom = dialog.header.mapToItem(
                    dialog.background, 0, dialog.header.height).y
        var contentTop = dialog.contentItem.mapToItem(
                    dialog.background, 0, 0).y
        compare(contentTop - headerBottom, dialog.spacing,
                "标题和正文之间只保留 Dialog.spacing")

        var footerStatus = findChild(dialog, "dialogFooterStatus")
        verify(footerStatus.visible)
        compare(footerStatus.text,
                "支持磁力 / ed2k / HTTP；可拖入或选择 .torrent 文件。")

        var input = findChild(dialog, "newTaskInput")
        verify(input)
        compare(input.wrapMode, TextEdit.NoWrap,
                "离线任务输入框不能把单条链接软换行")
        input.text = "https://example.com/" + "a".repeat(800)
        tryVerify(function () {
            return input.width > dialog.contentItem.width
        }, 1000, "超长链接应横向展开，而不是被压回弹窗宽度")
        var scroll = findChild(dialog, "newTaskInputScroll")
        var horizontalBar = findChild(dialog, "newTaskInputHorizontalBar")
        var verticalBar = findChild(dialog, "newTaskInputVerticalBar")
        verify(scroll && horizontalBar && verticalBar)
        compare(horizontalBar.visible, true)
        compare(verticalBar.visible, false)
        compare(horizontalBar.y + horizontalBar.height, scroll.height,
                "横向滚动条应贴在输入框底部，而不是落在左上角")
        compare(horizontalBar.width, scroll.width)
        compare(horizontalBar.height, 6, "横向滚动条只保留 6px 滑块车道")
        compare(horizontalBar.background, null, "不要 KDE 原生整条轨道线")
        compare(scroll.availableHeight, scroll.height - horizontalBar.height,
                "输入区应只给横向滚动条让出 6px 车道")

        input.text = "https://example.com/a\n".repeat(30)
        tryVerify(function () {
            return verticalBar.visible
        }, 1000, "多行链接应显示右侧竖向滚动条")
        compare(verticalBar.x + verticalBar.width, scroll.width)
        compare(verticalBar.height, scroll.height)

        var text = "magnet:?xt=urn:btih:first\n\nhttps://example.com/second.zip"
        input.text = text
        dialog.submit()

        compare(backend.submitted, 1)
        compare(backend.submittedText, text)
        compare(backend.submittedFolderId, "")
        compare(backend.submittedToFolder, false)
        compare(dialog.opened, false)
    }

    function test_new_task_dialog_can_choose_and_preview_torrent() {
        page.openNewTaskDialog("folder-current", "全部文件 / 动画")

        var dialog = findChild(page, "newTaskDialog")
        verify(dialog)
        tryVerify(function () {
            return dialog.opened
        })

        var choose = findChild(dialog, "newTaskChooseTorrent")
        verify(choose)
        verify(choose.enabled)

        var drop = findChild(dialog, "newTaskTorrentDropArea")
        verify(drop)

        backend.torrentParseFailed("旧错误")
        tryCompare(findChild(dialog, "dialogFooterStatus"), "text", "旧错误")
        mouseClick(choose)
        compare(backend.torrentPickerCalls, 1)
        compare(findChild(dialog, "dialogFooterStatus").text,
                "支持磁力 / ed2k / HTTP；可拖入或选择 .torrent 文件。")

        backend.torrentParsed("示例合集", "magnet:?xt=urn:btih:abc", "3 个文件 · 2 MiB")
        tryCompare(findChild(dialog, "newTaskInput"), "text",
                   "magnet:?xt=urn:btih:abc")
        compare(findChild(dialog, "dialogFooterStatus").text,
                "已解析「示例合集」：3 个文件 · 2 MiB，已加入输入框。")

        backend.torrentParsed("第二个", "magnet:?xt=urn:btih:def", "单文件 · 1 KiB")
        tryCompare(findChild(dialog, "newTaskInput"), "text",
                   "magnet:?xt=urn:btih:abc\nmagnet:?xt=urn:btih:def")

        dialog.submit()
        compare(backend.submitted, 1)
        compare(backend.submittedText,
                "magnet:?xt=urn:btih:abc\nmagnet:?xt=urn:btih:def")
        compare(backend.submittedFolderId, "folder-current")
        compare(backend.submittedToFolder, true)
        compare(dialog.opened, false)
    }

    function test_new_task_dialog_forwards_dropped_torrent_urls_and_errors() {
        page.openNewTaskDialog()

        var dialog = findChild(page, "newTaskDialog")
        verify(dialog)
        tryVerify(function () {
            return dialog.opened
        })

        dialog.previewTorrentUrls([
            "file:///tmp/a.torrent",
            "file:///tmp/b.torrent"
        ])
        compare(backend.torrentPreviewCalls, 1)
        compare(backend.torrentPreviewUrls.length, 2)
        compare(backend.torrentPreviewUrls[0], "file:///tmp/a.torrent")
        compare(backend.torrentPreviewUrls[1], "file:///tmp/b.torrent")

        backend.torrentParseFailed("坏种子.torrent：缺少 info 字典")
        tryCompare(findChild(dialog, "dialogFooterStatus"), "text",
                   "坏种子.torrent：缺少 info 字典")
        verify(findChild(dialog, "dialogFooterStatus").color ===
               dialog.theme.danger)
    }

    function test_new_task_dialog_can_target_current_folder() {
        page.openNewTaskDialog("folder-current", "全部文件 / 动画")

        var dialog = findChild(page, "newTaskDialog")
        verify(dialog)
        tryVerify(function () {
            return dialog.opened
        })
        compare(dialog.targetFolderId, "folder-current")
        compare(dialog.targetFolderPath, "全部文件 / 动画")

        var destinationRow = findChild(dialog, "newTaskDestinationRow")
        var destinationPath = findChild(dialog, "newTaskDestinationPath")
        verify(destinationRow && destinationRow.visible)
        verify(destinationPath)
        compare(destinationPath.text, "保存到：全部文件 / 动画")

        var input = findChild(dialog, "newTaskInput")
        verify(input)
        input.text = "magnet:?xt=urn:btih:target"
        dialog.submit()

        compare(backend.submitted, 1)
        compare(backend.submittedText, "magnet:?xt=urn:btih:target")
        compare(backend.submittedFolderId, "folder-current")
        compare(backend.submittedToFolder, true)
        compare(dialog.opened, false)
    }

    function test_new_task_dialog_can_target_file_page_root() {
        page.openNewTaskDialog("", "全部文件")

        var dialog = findChild(page, "newTaskDialog")
        verify(dialog)
        tryVerify(function () {
            return dialog.opened
        })
        verify(dialog.saveToFolder)

        var input = findChild(dialog, "newTaskInput")
        verify(input)
        input.text = "https://example.com/root.zip"
        dialog.submit()

        compare(backend.submitted, 1)
        compare(backend.submittedFolderId, "")
        compare(backend.submittedToFolder, true)
    }

    function test_new_task_dialog_can_choose_another_save_folder() {
        page.openNewTaskDialog()

        var dialog = findChild(page, "newTaskDialog")
        verify(dialog)
        tryVerify(function () {
            return dialog.opened
        })
        compare(findChild(dialog, "newTaskDestinationPath").text,
                "保存到：My Pack")

        var change = findChild(dialog, "newTaskChangeDestination")
        verify(change)
        mouseClick(change)
        wait(40)

        var picker = findChild(dialog, "taskFolderPickerDialog")
        verify(picker)
        tryVerify(function () {
            return picker.opened
        })
        compare(picker.targetPathText, "全部文件")

        picker.enterFolder({ id: "A", name: "动画" })
        wait(30)
        compare(picker.targetPathText, "全部文件 / 动画")

        picker.enterFolder({ id: "B", name: "2026" })
        wait(30)
        compare(picker.targetPathText, "全部文件 / 动画 / 2026")

        var pickerOk = findChild(picker, "dialogFooterOk")
        verify(pickerOk)
        verify(pickerOk.enabled)
        mouseClick(pickerOk)
        wait(40)

        verify(!picker.opened)
        compare(dialog.targetFolderId, "B")
        compare(dialog.targetFolderPath, "全部文件 / 动画 / 2026")
        compare(dialog.pickerStartPath.length, 3)
        compare(dialog.pickerStartPath[2].id, "B")
        compare(findChild(dialog, "newTaskDestinationPath").text,
                "保存到：全部文件 / 动画 / 2026")

        var input = findChild(dialog, "newTaskInput")
        input.text = "https://example.com/another.zip"
        dialog.submit()

        compare(backend.submitted, 1)
        compare(backend.submittedFolderId, "B")
        compare(backend.submittedToFolder, true)
    }

    function test_folder_picker_starts_at_the_file_page_directory() {
        page.openNewTaskDialog("A", "全部文件 / 动画", [
            { id: "", name: "全部文件" },
            { id: "A", name: "动画" }
        ])

        var dialog = findChild(page, "newTaskDialog")
        verify(dialog)
        tryVerify(function () {
            return dialog.opened
        })

        mouseClick(findChild(dialog, "newTaskChangeDestination"))
        wait(40)

        var picker = findChild(dialog, "taskFolderPickerDialog")
        verify(picker)
        tryVerify(function () {
            return picker.opened
        })
        compare(picker.targetId, "A")
        compare(picker.targetPathText, "全部文件 / 动画")

        picker.close()
        wait(20)
        dialog.close()
    }

    function test_activation_refreshes_tasks() {
        page.active = false
        backend.refreshCalls = 0
        compare(backend.refreshCalls, 0)

        page.active = true
        tryCompare(backend, "refreshCalls", 1)

        page.active = false
        page.active = true
        tryCompare(backend, "refreshCalls", 2)
    }

    function test_header_refresh_feedback_keeps_actions_only() {
        push([task("t1", "running", 10), task("t2", "completed", 100)])
        var refresh = findChild(taskHeader, "tasksRefreshButton")
        verify(refresh !== null)
        var spinner = findChild(refresh, "buttonBusyIndicator")
        verify(spinner !== null)
        compare(refresh.text, "刷新")
        compare(refresh.enabled, true)
        compare(spinner.running, false)
        var idleWidth = refresh.width

        backend.tasksLoading = true
        backend.changed()
        wait(20)
        compare(refresh.text, "刷新")
        compare(refresh.enabled, false)
        compare(spinner.running, true)
        compare(refresh.width, idleWidth)

        backend.tasksLoading = false
        backend.changed()
        wait(20)
        compare(refresh.text, "刷新")
        compare(refresh.enabled, true)
        compare(spinner.running, false)
        compare(refresh.width, idleWidth)
    }

    function test_clear_finished_button_targets_only_terminal_rows() {
        var button = findChild(taskHeader, "clearFinishedTasksButton")
        verify(button !== null)
        compare(taskHeader.showClearFinished, false)

        push([
            task("running", "running", 20),
            task("pending", "pending", 0),
            task("paused", "paused", 10),
            task("unknown", "unknown", 0)
        ])
        compare(page.finishedCount, 0)
        compare(taskHeader.showClearFinished, false,
                "排队 / 下载中 / 暂停 / 未知状态都不能显示批量清理")

        push([
            task("done-a", "completed", 100),
            task("done-b", "completed", 100),
            task("failed", "error", 0),
            task("deleted", "deleted", 0),
            task("running", "running", 20)
        ])
        compare(page.finishedCount, 4)
        compare(taskHeader.finishedCount, 4)
        compare(taskHeader.backendReady, true)
        compare(taskHeader.showClearFinished, true)
        compare(button.text, "清理已结束任务")
        compare(button.enabled, true)
        var newButton = findChild(taskHeader, "newTaskButton")
        var refreshButton = findChild(taskHeader, "tasksRefreshButton")
        verify(taskHeader.children.indexOf(newButton) < taskHeader.children.indexOf(button),
               "清理已结束任务应排在新任务之后")
        verify(taskHeader.children.indexOf(button) < taskHeader.children.indexOf(refreshButton),
               "刷新应保持在最右侧")

        backend.tasksClearing = true
        backend.changed()
        wait(20)
        compare(button.enabled, false, "清理期间阻止重复触发")
        compare(findChild(button, "buttonBusyIndicator").running, true)

        backend.tasksClearing = false
        backend.changed()
        wait(20)
        compare(button.enabled, true)
    }

    function test_clear_finished_uses_one_batch_confirmation() {
        push([
            task("done-a", "completed", 100),
            task("failed", "error", 0),
            task("deleted", "deleted", 0),
            task("running", "running", 20)
        ])

        var button = findChild(taskHeader, "clearFinishedTasksButton")
        verify(button !== null)
        verify(taskHeader.showClearFinished)
        button.clicked()
        wait(40)

        var dialog = findChild(page, "clearFinishedTasksDialog")
        verify(dialog !== null && dialog.opened)
        compare(backend.clearFinishedCalls, 0, "确认前不能删除记录")
        var text = findChild(dialog, "clearFinishedTasksText")
        verify(text.text.indexOf("3 条") >= 0, text.text)
        verify(text.text.indexOf("已完成、失败和产物已删除") >= 0, text.text)
        verify(text.text.indexOf("云端文件不会删除") >= 0, text.text)

        var footer = findChild(dialog, "clearFinishedTasksFooter")
        verify(footer !== null)
        mouseClick(findChild(footer, "dialogFooterOk"))
        wait(40)
        compare(backend.clearFinishedCalls, 1, "一次确认只发一次批量清理")
        compare(dialog.opened, false)
    }

    function test_empty_loading_uses_skeleton_instead_of_message() {
        compare(list.count, 0)
        compare(backend.tasksLoading, false)
        compare(page.tasksLoading, false)
        compare(page.showEmptyState, true)
        compare(page.showLoadingSkeleton, false)

        backend.tasksLoading = true
        compare(page.showLoadingSkeleton, true)
        compare(page.showLoadingState, false)
        compare(page.showEmptyState, false)

        backend.tasksLoading = false
        compare(page.showLoadingSkeleton, false)
        compare(page.showLoadingState, false)
        compare(page.showEmptyState, true)
    }

    function test_field_change_patches_in_place_and_keeps_scroll() {
        var many = []
        for (var i = 0; i < 30; i++)
            many.push(task("row" + i, i === 0 ? "running" : "completed", 0))
        push(many)
        compare(model.count, 30)

        list.contentY = 240
        tryVerify(function () {
            return list.contentY === 240
        })

        // 只有进度变化：应当原地打补丁，行数与滚动位置都不动
        many[0].progress = 42
        many[0].progressText = "42%"
        push(many.slice())

        compare(model.count, 30, "进度变化不该增删行")
        compare(model.get(0).progress, 42)
        compare(model.get(0).progressText, "42%")
        compare(list.contentY, 240, "局部更新不能把滚动位置冲回顶部")
    }

    function test_order_follows_rust_when_a_task_settles() {
        push([
            task("x", "running", 50),
            task("y", "pending", 0),
            task("z", "running", 10)
        ])
        compare(model.get(0).key, "x")

        // Rust 侧排序：进行中的在前，已结束的挪到后面
        push([
            task("z", "running", 10),
            task("y", "pending", 0),
            task("x", "completed", 100)
        ])
        compare(model.count, 3)
        compare(model.get(0).key, "z")
        compare(model.get(1).key, "y")
        compare(model.get(2).key, "x")
        compare(model.get(2).settled, true, "状态字段也要跟着走")
    }

    function test_removed_task_disappears() {
        push([task("a", "completed", 100), task("b", "completed", 100)])
        compare(model.count, 2)

        push([task("b", "completed", 100)])
        compare(model.count, 1)
        compare(model.get(0).key, "b")
    }

    function test_empty_backend_clears_rows() {
        push([task("only", "completed", 100)])
        compare(model.count, 1)

        push([])
        compare(model.count, 0, "登出 / 清空后列表要空，不能留下上一批的行")
    }

    function test_inactive_page_releases_rows_and_restores_scroll() {
        var many = []
        for (var i = 0; i < 30; ++i)
            many.push(task("release-" + i, i === 0 ? "running" : "completed", 0))
        push(many)
        compare(model.count, 30)
        list.contentY = 240
        tryVerify(function () { return list.contentY === 240 })

        page.active = false
        tryCompare(model, "count", 0)
        push(many.slice())
        compare(model.count, 0, "隐藏页收到轮询更新时不能重新建行")

        page.active = true
        tryCompare(model, "count", 30)
        tryVerify(function () { return list.contentY > 0 },
                  2000, "回到离线任务页后应恢复原滚动位置")
    }
}
