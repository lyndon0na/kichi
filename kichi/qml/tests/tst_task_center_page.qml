import QtQuick
import QtTest
import ".."

// 任务中心壳层：一个入口、两张独立表；切标签只改变哪张表处于 active。
TestCase {
    name: "TaskCenterPage"
    when: windowShown
    width: 1000
    height: 700

    QtObject {
        id: backend

        property var transfers: []
        property int transferRevision: 0
        property int downloadActive: 0
        property int downloadQueued: 0
        property int uploadActive: 0
        property int uploadQueued: 0
        property var transferLive: []
        property var transferDetail: []
        property bool transferDetailActive: false
        property bool transferHasFinished: false
        property bool transferHasRetryable: false

        property var tasks: []
        property bool tasksLoading: false
        property bool tasksClearing: false
        property bool playLoading: false
        property string playLoadingId: ""
        property var playingIds: []
        property string taskFolderLoadingId: ""
        property string authState: "signedIn"

        property var moveTargets: []
        property int moveTargetsRevision: 0
        property bool moveTargetsLoading: false
        property string moveTargetsError: ""

        property int ensureCalls: 0
        property int refreshCalls: 0

        signal changed()
        signal thumbnailReady(string id, string path)
        signal torrentParsed(string name, string magnet, string detail)
        signal torrentParseFailed(string message)

        function ensureTasksFresh() {
            ensureCalls += 1
        }

        function refreshTasks() {
            refreshCalls += 1
        }

        function openTransferFolder(key) {}
        function closeTransferFolder() {}
        function cancelTransfer(key) {}
        function pauseTransfer(key) {}
        function resumeTransfer(key) {}
        function retryTransfer(key) {}
        function retryFailedTransfers() {}
        function removeTransfer(key) { return true }
        function revealTransfer(key) {}
        function clearFinishedTransfers() {}
        function cancelAllTransfers() {}

        function addTask(text) {}
        function addTaskToFolder(text, folderId) {}
        function deleteTask(id) {}
        function clearFinishedTasks() {}
        function thumbnailPath(id) { return "" }
        function requestThumbnail(id) {}
        function cancelPlayLoading() {}
        function currentPath() { return [] }
        function loadMoveTargets(parentId) {}
        function cancelMoveTargets() {}
    }

    Component {
        id: pageComponent

        TaskCenterPage {}
    }

    property var page: null

    function init() {
        backend.transfers = []
        backend.transferRevision = 0
        backend.downloadActive = 0
        backend.downloadQueued = 0
        backend.uploadActive = 0
        backend.uploadQueued = 0
        backend.transferLive = []
        backend.transferDetail = []
        backend.transferDetailActive = false
        backend.transferHasFinished = false
        backend.transferHasRetryable = false
        backend.tasks = []
        backend.tasksLoading = false
        backend.tasksClearing = false
        backend.playLoading = false
        backend.playLoadingId = ""
        backend.playingIds = []
        backend.taskFolderLoadingId = ""
        backend.moveTargets = []
        backend.moveTargetsRevision = 0
        backend.moveTargetsLoading = false
        backend.moveTargetsError = ""
        backend.ensureCalls = 0
        backend.refreshCalls = 0

        page = pageComponent.createObject(parent, {
            width: 1000,
            height: 700,
            visible: true,
            active: true,
            backend: backend
        })
        verify(page !== null, "TaskCenterPage 创建失败")
        wait(20)
    }

    function cleanup() {
        if (page)
            page.destroy()
        page = null
    }

    function unfinishedTask() {
        return {
            id: "offline-1",
            name: "离线任务样本",
            kind: "file",
            phase: "RUNNING",
            phaseText: "进行中",
            progress: 0.5,
            progressText: "50%",
            sizeText: "1 MiB",
            createdText: "今天",
            detail: "",
            settled: false,
            fileId: "",
            hasThumbnail: false,
            video: false,
            playable: false
        }
    }

    function test_defaults_to_transfers_with_only_one_active_table() {
        compare(page.currentTab, "transfers")
        compare(page.transfersView.active, true)
        compare(page.tasksView.active, false)
        compare(findChild(page, "taskTabTransfers").selected, true)
        compare(findChild(page, "taskTabOffline").selected, false)
    }

    function test_switching_tabs_activates_only_the_selected_table() {
        mouseClick(findChild(page, "taskTabOffline"))
        tryCompare(page, "currentTab", "offline")
        compare(page.transfersView.active, false)
        compare(page.tasksView.active, true)
        compare(findChild(page, "taskTabOffline").selected, true)
        verify(backend.ensureCalls >= 1, "切到离线标签应触发现有的后台校准入口")

        mouseClick(findChild(page, "taskTabTransfers"))
        tryCompare(page, "currentTab", "transfers")
        compare(page.transfersView.active, true)
        compare(page.tasksView.active, false)
    }

    function test_tab_counts_include_unfinished_items() {
        backend.downloadActive = 2
        backend.uploadActive = 1
        backend.tasks = [
            { settled: false },
            { settled: false },
            { settled: true }
        ]
        backend.changed()
        wait(20)

        compare(page.transferActiveCount, 3)
        compare(page.offlineActiveCount, 2)
        compare(findChild(page, "taskTabTransfersCount").text, "3")
        compare(findChild(page, "taskTabOfflineCount").text, "2")
    }

    function test_show_helpers_select_the_requested_table() {
        page.showOffline()
        compare(page.currentTab, "offline")
        compare(page.tasksView.active, true)

        page.showTransfers()
        compare(page.currentTab, "transfers")
        compare(page.transfersView.active, true)
    }

    function test_tab_labels_are_horizontally_centered() {
        const transfersTab = findChild(page, "taskTabTransfers")
        const transfersLabel = findChild(page, "taskTabTransfersLabel")
        const transferOffset = transfersLabel.mapToItem(
                    transfersTab, transfersLabel.width / 2, 0).x
                                - transfersTab.width / 2
        verify(Math.abs(transferOffset) <= 0.5,
               "本机传输文字横向偏移 " + transferOffset + "px")

        const offlineTab = findChild(page, "taskTabOffline")
        const offlineLabel = findChild(page, "taskTabOfflineLabel")
        const offlineOffset = offlineLabel.mapToItem(
                    offlineTab, offlineLabel.width / 2, 0).x
                              - offlineTab.width / 2
        verify(Math.abs(offlineOffset) <= 0.5,
               "离线任务文字横向偏移 " + offlineOffset + "px")
    }

    function test_selection_indicator_tracks_the_active_tab() {
        const indicator = findChild(page, "taskTabSelectionIndicator")
        const transfersTab = findChild(page, "taskTabTransfers")
        const offlineTab = findChild(page, "taskTabOffline")

        tryCompare(indicator, "x",
                   transfersTab.mapToItem(indicator.parent, 0, 0).x, 1000)
        compare(indicator.width, transfersTab.width)

        mouseClick(offlineTab)
        tryCompare(page, "currentTab", "offline")
        // 选中态字重不同，会让两个标签宽度差 1px 并引起布局位移；
        // 期望位置必须按布局稳定后的实时值算，不能在切换瞬间取快照。
        tryVerify(function () {
            return Math.abs(indicator.x
                            - offlineTab.mapToItem(indicator.parent, 0, 0).x) <= 0.5
                   && Math.abs(indicator.width - offlineTab.width) <= 0.5
        }, 1000, "选中指示器没有跟住离线任务标签")
    }

    function test_task_entry_prefers_local_transfers_when_both_have_tasks() {
        backend.downloadActive = 1
        backend.tasks = [unfinishedTask()]
        backend.changed()
        wait(20)
        page.showOffline()

        page.showTaskEntry()

        compare(page.currentTab, "transfers")
    }

    function test_task_entry_opens_offline_when_only_offline_has_tasks() {
        backend.downloadActive = 0
        backend.uploadActive = 0
        backend.tasks = [unfinishedTask()]
        backend.changed()
        wait(20)
        page.showTransfers()

        page.showTaskEntry()

        compare(page.currentTab, "offline")
    }

    function test_task_entry_opens_transfers_when_only_local_has_tasks() {
        backend.downloadActive = 1
        backend.uploadActive = 0
        backend.tasks = []
        backend.changed()
        wait(20)
        page.showOffline()

        page.showTaskEntry()

        compare(page.currentTab, "transfers")
    }

    function test_task_entry_defaults_to_transfers_without_tasks() {
        backend.downloadActive = 0
        backend.uploadActive = 0
        backend.tasks = []
        backend.changed()
        wait(20)
        page.showOffline()

        page.showTaskEntry()

        compare(page.currentTab, "transfers")
    }
}
