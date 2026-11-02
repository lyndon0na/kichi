import QtQuick
import QtQuick.Controls
import QtTest
import ".."

// 传输任务单列表回归测试：一行同时承载进行中 / 排队 / 历史，
// 文件夹子行只按需内联插入，响应式列与逐行动作都直接驱动真实组件。
TestCase {
    name: "TransfersPageRows"
    when: windowShown
    width: 1280
    height: 720

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

        property var detailSets: ({})
        property int openCount: 0
        property int closeCount: 0
        property string openedKey: ""
        property int cancelCount: 0
        property string cancelledKey: ""
        property int pauseCount: 0
        property string pausedKey: ""
        property int resumeCount: 0
        property string resumedKey: ""
        property int retryCount: 0
        property string retriedKey: ""
        property int retryFailedCount: 0
        property int removeCount: 0
        property string removedKey: ""
        property int revealCount: 0
        property string revealedKey: ""
        property int clearCount: 0
        property int cancelAllCount: 0

        signal changed()

        function reset() {
            transfers = []
            transferRevision = 0
            downloadActive = 0
            downloadQueued = 0
            uploadActive = 0
            uploadQueued = 0
            transferLive = []
            transferDetail = []
            transferDetailActive = false
            transferHasFinished = false
            transferHasRetryable = false
            detailSets = ({})
            openCount = 0
            closeCount = 0
            openedKey = ""
            cancelCount = 0
            cancelledKey = ""
            pauseCount = 0
            pausedKey = ""
            resumeCount = 0
            resumedKey = ""
            retryCount = 0
            retriedKey = ""
            retryFailedCount = 0
            removeCount = 0
            removedKey = ""
            revealCount = 0
            revealedKey = ""
            clearCount = 0
            cancelAllCount = 0
        }

        function publish(items, active, queued) {
            transfers = items
            downloadActive = active === undefined ? 0 : active
            downloadQueued = queued === undefined ? 0 : queued
            transferHasFinished = items.some(function(item) {
                return item.state === "done" || item.state === "failed"
                        || item.state === "cancelled"
            })
            transferHasRetryable = items.some(function(item) {
                return item.state === "failed"
                        && (item.folder !== true || item.childCount === 0)
            })
            transferRevision += 1
            changed()
        }

        function setLive(items) {
            transferLive = items
            changed()
        }

        function setDetails(key, children) {
            var next = detailSets
            next[key] = children
            detailSets = next
        }

        function openTransferFolder(key) {
            openCount += 1
            openedKey = key
            if (detailSets[key] !== undefined) {
                transferDetail = detailSets[key]
                transferDetailActive = true
            } else {
                transferDetail = []
                transferDetailActive = false
            }
            transferRevision += 1
            changed()
        }

        function closeTransferFolder() {
            closeCount += 1
            transferDetail = []
            transferDetailActive = false
            transferRevision += 1
            changed()
        }

        function invalidateTransferDetail() {
            transferDetail = []
            transferDetailActive = false
            transferRevision += 1
            changed()
        }

        function cancelTransfer(key) {
            cancelCount += 1
            cancelledKey = key
        }

        function pauseTransfer(key) {
            pauseCount += 1
            pausedKey = key
        }

        function resumeTransfer(key) {
            resumeCount += 1
            resumedKey = key
        }

        function retryTransfer(key) {
            retryCount += 1
            retriedKey = key
        }

        function retryFailedTransfers() {
            retryFailedCount += 1
        }

        function removeTransfer(key) {
            removeCount += 1
            removedKey = key
            return true
        }

        function revealTransfer(key) {
            revealCount += 1
            revealedKey = key
        }

        function clearFinishedTransfers() {
            clearCount += 1
        }

        function cancelAllTransfers() {
            cancelAllCount += 1
        }

    }

    Component {
        id: pageComponent
        TransfersPage {}
    }

    Component {
        id: headerComponent
        TransferHeaderControls {}
    }

    property var page: null
    property var header: null
    property var list: null

    function makeRow(key, name, state, extra) {
        var row = {
            key: key,
            name: name,
            sourceId: "source-" + key,
            state: state,
            stateText: stateText(state),
            running: state === "running",
            progress: 0,
            progressText: "",
            sizeText: "10 B",
            detail: "",
            path: "",
            folder: false,
            childCount: 0,
            failedCount: 0
        }
        var fields = extra || {}
        for (var field in fields)
            row[field] = fields[field]
        return row
    }

    function stateText(state) {
        if (state === "queued")
            return "等待中"
        if (state === "preparing")
            return "准备中"
        if (state === "running")
            return "下载中"
        if (state === "pausing")
            return "正在暂停"
        if (state === "paused")
            return "已暂停"
        if (state === "done")
            return "已完成"
        if (state === "failed")
            return "失败"
        if (state === "cancelled")
            return "已取消"
        if (state === "interrupted")
            return "已中断"
        return "提示"
    }

    function row(key) {
        for (var i = 0; list && i < list.count; i++) {
            var item = list.itemAtIndex(i)
            if (item && item.key === key)
                return item
        }
        return findChild(page, "transferRow_" + key)
    }

    function waitRows(count) {
        tryVerify(function() {
            return list !== null && list.count === count
        }, 2000, "期望 " + count + " 行，实际 " + (list ? list.count : -1))
        wait(60)
    }

    function init() {
        backend.reset()
        page = pageComponent.createObject(parent, {
            width: 1200,
            height: 640,
            visible: true,
            viewportWidth: 1280,
            backend: backend
        })
        verify(page !== null, "TransfersPage 创建失败")
        header = headerComponent.createObject(parent, {
            width: 720,
            height: 40,
            visible: true,
            target: page
        })
        verify(header !== null, "TransferHeaderControls 创建失败")
        list = findChild(page, "transferList")
        verify(list !== null, "transferList 不存在")
        wait(20)
    }

    function cleanup() {
        if (header)
            header.destroy()
        if (page)
            page.destroy()
        header = null
        page = null
        list = null
    }

    function test_one_table_holds_all_states() {
        backend.publish([
            makeRow("1:0", "正在下载.mkv", "running", {
                progress: 35,
                progressText: "35%",
                sizeText: "35 MB / 100 MB"
            }),
            makeRow("2:0", "排队.mkv", "queued"),
            makeRow("3:0", "完成.mkv", "done", {
                progress: 100,
                path: "/tmp/完成.mkv"
            }),
            makeRow("4:0", "失败.mkv", "failed", {
                detail: "连接超时"
            })
        ], 2, 1)

        waitRows(4)
        compare(findChild(page, "transferTableHeader") !== null, true)
        compare(findChild(page, "transferActiveSummary"), null)
        compare(findChild(page, "queueSectionHeader"), null)
        compare(findChild(page, "historySectionHeader"), null)
        compare(findChild(page, "transferDetailList"), null)
        compare(list.count, 4, "所有状态只在同一张表里出现")
        compare(findChild(header, "transferHeaderStatus"), null,
                "传输任务标题旁不再显示状态计数")
    }

    function test_header_count_properties_count_queued_rows_only_once() {
        backend.publish([
            makeRow("1:0", "a.mkv", "queued"),
            makeRow("2:0", "b.mkv", "queued"),
            makeRow("3:0", "c.mkv", "queued")
        ], 3, 3)
        waitRows(3)
        compare(header.activeCount, 3)
        compare(header.queuedCount, 3)

        backend.publish([
            makeRow("1:0", "a.mkv", "running"),
            makeRow("2:0", "b.mkv", "queued"),
            makeRow("3:0", "c.mkv", "queued"),
            makeRow("4:0", "d.mkv", "queued")
        ], 4, 3)
        waitRows(4)
        compare(header.activeCount, 4)
        compare(header.queuedCount, 3)
    }

    function test_rows_and_header_use_fixed_density() {
        backend.publish([
            makeRow("1:0", "a.mkv", "running", {
                progress: 50,
                progressText: "50%"
            })
        ], 1, 0)
        waitRows(1)

        var rowItem = row("1:0")
        var tableHeader = findChild(page, "transferTableHeader")
        verify(rowItem !== null)
        verify(tableHeader !== null)
        compare(rowItem.implicitHeight, 48)
        compare(tableHeader.implicitHeight, 34)
        compare(findChild(page, "nameText").text, "a.mkv")
        compare(findChild(page, "stateText").text, "下载中")
        compare(findChild(page, "sizeText").visible, true)
        compare(findChild(page, "rateText").visible, true)
    }

    function test_type_icons_align_without_moving_child_indent() {
        backend.setDetails("9:folder", [
            makeRow("9:0", "子文件.mkv", "done", { progress: 100 })
        ])
        backend.publish([
            makeRow("1:0", "普通文件.mkv", "done", { progress: 100 }),
            makeRow("9:folder", "目录", "done", {
                folder: true,
                childCount: 1
            })
        ], 0, 0)
        waitRows(2)

        var fileIcon = findChild(row("1:0"), "rowTypeIcon")
        var folderIcon = findChild(row("9:folder"), "rowTypeIcon")
        verify(fileIcon !== null)
        verify(folderIcon !== null)
        compare(fileIcon.x, folderIcon.x, "顶层普通文件与目录图标必须同一竖线")

        mouseClick(findChild(row("9:folder"), "rowHeader"))
        waitRows(3)
        var childIcon = findChild(row("9:0"), "rowTypeIcon")
        verify(childIcon !== null)
        compare(childIcon.x, 28, "子文件图标保持现有缩进")
    }

    function test_failure_reason_is_merged_into_status() {
        backend.publish([
            makeRow("1:0", "坏文件.mkv", "failed", {
                detail: "连接超时"
            })
        ], 0, 0)
        waitRows(1)

        var rowItem = row("1:0")
        verify(rowItem !== null)
        compare(findChild(rowItem, "stateText").text, "失败 · 连接超时")
        compare(findChild(rowItem, "detailLink"), null)
        compare(rowItem.implicitHeight, 48, "失败原因并入单行状态列")
        compare(findChild(rowItem, "retryLink").visible, true)
    }

    function test_folder_expands_inline_and_only_one_stays_open() {
        backend.setDetails("10:folder", [
            makeRow("10:0", "第一集.mkv", "done", {
                path: "/tmp/剧集/第一集.mkv",
                progress: 100
            }),
            makeRow("10:1", "第二集.mkv", "failed", {
                detail: "断网"
            })
        ])
        backend.setDetails("11:folder", [
            makeRow("11:0", "花絮.mkv", "done", { progress: 100 })
        ])
        backend.publish([
            makeRow("10:folder", "剧集", "failed", {
                folder: true,
                childCount: 2,
                failedCount: 1,
                detail: "成功 1 个 · 失败 1 个"
            }),
            makeRow("11:folder", "花絮目录", "done", {
                folder: true,
                childCount: 1
            })
        ], 0, 0)
        waitRows(2)
        compare(backend.detailSets["10:folder"].length, 2)

        var first = row("10:folder")
        verify(first !== null && first.expandable)
        mouseClick(findChild(first, "rowHeader"))
        compare(backend.transferDetail.length, 2, "后端应返回两个子文件")
        compare(page.wantedRows().length, 4, "扁平化模型应包含父行与两个子行")
        waitRows(4)
        compare(page.expandedKey, "10:folder")
        compare(backend.openedKey, "10:folder")
        compare(list.cacheBuffer, 0, "展开期间关闭缓存，避免重挂模型刷 Qt 告警")
        compare(row("10:0").depth, 1)
        compare(row("10:0").rowKind, "child")
        compare(row("10:0").parentKey, "10:folder")
        compare(findChild(row("10:0"), "stateText").text, "已完成")
        compare(row("10:1").implicitHeight, 48)

        var second = row("11:folder")
        mouseClick(findChild(second, "rowHeader"))
        waitRows(3)
        compare(page.expandedKey, "11:folder")
        compare(backend.openedKey, "11:folder")
        compare(row("10:0"), null, "展开另一个文件夹时先收起前一个")
        verify(row("11:0") !== null)

        mouseClick(findChild(row("11:folder"), "openDetailsArrowHit"))
        waitRows(2)
        compare(page.expandedKey, "")
        compare(backend.closeCount, 1)
        compare(list.cacheBuffer, 400, "收起文件夹后恢复委托预加载")
    }

    function test_preparing_or_empty_folder_cannot_expand() {
        backend.setDetails("20:folder", [
            makeRow("20:0", "a.mkv", "queued")
        ])
        backend.publish([
            makeRow("20:folder", "准备中目录", "preparing", {
                folder: true,
                childCount: 1
            }),
            makeRow("21:folder", "空目录", "done", {
                folder: true,
                childCount: 0
            })
        ], 1, 0)
        waitRows(2)

        var preparing = row("20:folder")
        var empty = row("21:folder")
        compare(preparing.expandable, false)
        compare(empty.expandable, false)
        compare(findChild(preparing, "rowHeader").enabled, false)

        page.toggleFolder("20:folder")
        page.toggleFolder("21:folder")
        compare(backend.openCount, 0)
        compare(list.count, 2)
    }

    function test_expanded_parent_sticks_inside_child_range_and_restores() {
        var children = []
        for (var child = 0; child < 12; child++) {
            children.push(makeRow("60:" + child, "第" + (child + 1) + "集.mkv",
                                  "done", { progress: 100 }))
        }
        backend.setDetails("60:folder", children)

        var items = [
            makeRow("before:1", "前一项.mkv", "done", { progress: 100 }),
            makeRow("before:2", "前二项.mkv", "done", { progress: 100 }),
            makeRow("60:folder", "长目录", "done", {
                folder: true,
                childCount: children.length
            })
        ]
        for (var after = 0; after < 20; after++) {
            items.push(makeRow("after:" + after, "后续" + after + ".mkv",
                               "done", { progress: 100 }))
        }
        backend.publish(items, 0, 0)
        waitRows(23)

        mouseClick(findChild(row("60:folder"), "rowHeader"))
        waitRows(35)
        var sticky = findChild(page, "transferStickyRow")
        verify(sticky !== null)
        compare(sticky.visible, false, "父行自己还在顶部时不应重复覆盖")

        list.contentY = 100
        tryVerify(function() { return sticky.visible })
        compare(sticky.y, list.y)
        compare(findChild(sticky, "nameText").text, "长目录")

        mouseClick(sticky, 120, sticky.height / 2, Qt.RightButton)
        var menu = findChild(page, "transferContextMenu")
        tryVerify(function() { return menu.opened })
        verify(findChild(menu, "transferRemoveItem").visible,
               "吸附父行必须能打开同一套右键菜单")
        menu.close()

        list.contentY = 720
        tryVerify(function() { return !sticky.visible })

        list.contentY = 480
        tryVerify(function() { return sticky.visible })
        var beforeWheel = list.contentY
        mouseWheel(sticky, sticky.width / 2, sticky.height / 2, 0, -120)
        tryVerify(function() { return list.contentY !== beforeWheel })
        mouseClick(findChild(sticky, "openDetailsArrowHit"))
        waitRows(23)
        compare(page.expandedKey, "")
        compare(backend.closeCount, 1)
        wait(500)
        compare(list.contentY, 96, "收起吸附层后列表应回到父行位置")
    }

    function test_switching_expanded_folder_keeps_clicked_row_in_place() {
        var firstChildren = []
        for (var first = 0; first < 12; first++) {
            firstChildren.push(makeRow("70:" + first, "第一组" + first + ".mkv",
                                       "done", { progress: 100 }))
        }
        backend.setDetails("70:folder", firstChildren)
        backend.setDetails("71:folder", [
            makeRow("71:0", "第二组 A.mkv", "done", { progress: 100 }),
            makeRow("71:1", "第二组 B.mkv", "done", { progress: 100 })
        ])

        var items = [
            makeRow("lead:0", "开头.mkv", "done", { progress: 100 }),
            makeRow("70:folder", "第一组", "done", {
                folder: true,
                childCount: firstChildren.length
            })
        ]
        for (var filler = 0; filler < 6; filler++) {
            items.push(makeRow("fill:" + filler, "间隔" + filler + ".mkv",
                               "done", { progress: 100 }))
        }
        items.push(makeRow("71:folder", "第二组", "done", {
            folder: true,
            childCount: 2
        }))
        for (var tail = 0; tail < 20; tail++) {
            items.push(makeRow("tail:" + tail, "尾部" + tail + ".mkv",
                               "done", { progress: 100 }))
        }
        backend.publish(items, 0, 0)
        waitRows(29)

        mouseClick(findChild(row("70:folder"), "rowHeader"))
        waitRows(41)
        list.contentY = 18 * 48
        wait(40)
        var second = row("71:folder")
        verify(second !== null)
        var beforeY = second.y - list.contentY

        mouseClick(findChild(second, "rowHeader"))
        waitRows(31)
        second = row("71:folder")
        verify(second !== null)
        wait(40)
        second = row("71:folder")
        verify(second !== null)
        verify(Math.round(second.y - list.contentY) === Math.round(beforeY),
               "切换展开项时被点击的父行不能跳走：y=" + second.y
               + " contentY=" + list.contentY)
    }

    function test_backend_detail_clear_collapses_parent_row() {
        backend.setDetails("25:folder", [
            makeRow("25:0", "a.mkv", "done", { progress: 100 })
        ])
        backend.publish([
            makeRow("25:folder", "剧集", "done", {
                folder: true,
                childCount: 1
            })
        ], 0, 0)
        waitRows(1)

        mouseClick(findChild(row("25:folder"), "rowHeader"))
        waitRows(2)
        compare(page.expandedKey, "25:folder")
        compare(row("25:folder").expanded, true)

        backend.invalidateTransferDetail()
        waitRows(1)
        compare(page.expandedKey, "", "后端清理详情时父行必须同步收起")
        compare(row("25:folder").expanded, false)
        compare(row("25:0"), null)
    }

    function test_responsive_columns_follow_window_thresholds() {
        backend.publish([
            makeRow("1:0", "a.mkv", "running", {
                progress: 10,
                progressText: "10%",
                sizeText: "1 MB / 10 MB"
            })
        ], 1, 0)
        waitRows(1)

        var tableHeader = findChild(page, "transferTableHeader")
        var rowItem = row("1:0")
        page.viewportWidth = 1180
        compare(page.showRate, true)
        compare(page.showSize, true)
        compare(findChild(tableHeader, "transferRateHeader").visible, true)
        compare(findChild(rowItem, "rateText").visible, true)

        page.viewportWidth = 1179
        compare(page.showRate, false)
        compare(page.showSize, true)
        compare(findChild(tableHeader, "transferRateHeader").visible, false)
        compare(findChild(rowItem, "rateText").visible, false)
        compare(findChild(rowItem, "sizeText").visible, true)

        page.viewportWidth = 979
        compare(page.showRate, false)
        compare(page.showSize, false)
        compare(findChild(tableHeader, "transferSizeHeader").visible, false)
        compare(findChild(rowItem, "sizeText").visible, false)
    }

    function test_folder_and_child_actions_use_the_row_key() {
        backend.setDetails("30:folder", [
            makeRow("30:0", "坏文件.mkv", "failed", {
                detail: "断网",
                path: "/tmp/剧集/坏文件.mkv"
            }),
            makeRow("30:1", "排队.mkv", "queued")
        ])
        backend.publish([
            makeRow("30:folder", "剧集", "failed", {
                folder: true,
                childCount: 2,
                failedCount: 1,
                path: "/tmp/剧集"
            })
        ], 0, 0)
        waitRows(1)

        mouseClick(findChild(row("30:folder"), "rowHeader"))
        waitRows(3)
        var child = row("30:0")
        verify(child !== null)
        mouseClick(findChild(child, "retryLink"))
        wait(20)
        compare(backend.retriedKey, "30:0")
        mouseClick(findChild(child, "revealLink"))
        wait(20)
        compare(backend.revealedKey, "30:0")

        mouseClick(findChild(row("30:folder"), "revealLink"))
        wait(20)
        compare(backend.revealedKey, "30:folder")
    }

    function test_row_right_click_opens_state_specific_actions() {
        backend.publish([
            makeRow("80:0", "下载中.mkv", "running", {
                progress: 20,
                progressText: "20%"
            })
        ], 1, 0)
        waitRows(1)

        var running = row("80:0")
        verify(running !== null)
        mouseClick(running, 120, running.height / 2, Qt.RightButton)
        var menu = findChild(page, "transferContextMenu")
        verify(menu !== null)
        tryVerify(function() { return menu.opened })

        verify(findChild(menu, "transferPauseItem").visible)
        compare(findChild(menu, "transferResumeItem").visible, false)
        verify(findChild(menu, "transferCancelItem").visible)
        compare(findChild(menu, "transferRetryItem").visible, false)
        compare(findChild(menu, "transferRevealItem").visible, false)
        compare(findChild(menu, "transferRemoveItem").visible, false)

        mouseClick(findChild(menu, "transferPauseItem"))
        wait(20)
        compare(backend.pauseCount, 1)
        compare(backend.pausedKey, "80:0")

        backend.publish([
            makeRow("81:0", "已完成.mkv", "done", {
                progress: 100,
                path: "/tmp/已完成.mkv"
            })
        ], 0, 0)
        waitRows(1)
        mouseClick(row("81:0"), 120, 24, Qt.RightButton)
        tryVerify(function() { return menu.opened })
        compare(findChild(menu, "transferPauseItem").visible, false)
        verify(findChild(menu, "transferRevealItem").visible)
        verify(findChild(menu, "transferRemoveItem").visible)
    }

    function test_upload_rows_only_pause_before_start() {
        backend.publish([
            makeRow("84:0", "上传中.mkv", "running", {
                direction: "upload",
                stateText: "上传中",
                progress: 40,
                progressText: "40%",
                sizeText: "4 MB / 10 MB"
            }),
            makeRow("84:1", "排队上传.mkv", "queued", {
                direction: "upload",
                stateText: "排队中",
                sizeText: "3 MB"
            }),
            makeRow("84:2", "暂停上传.mkv", "paused", {
                direction: "upload",
                stateText: "已暂停",
                sizeText: "2 MB"
            }),
            makeRow("85:0", "上传失败.mkv", "failed", {
                direction: "upload",
                stateText: "失败",
                detail: "上传连接中断"
            })
        ], 1, 0)
        waitRows(4)

        var running = row("84:0")
        verify(running !== null)
        compare(findChild(running, "pauseLink").visible, false)
        compare(findChild(running, "resumeLink").visible, false)
        compare(findChild(running, "revealLink").visible, false)
        verify(findChild(running, "cancelLink").visible,
               "正在上传不能暂停，但必须能取消")

        var queued = row("84:1")
        verify(findChild(queued, "pauseLink").visible,
               "尚未开始的上传可以暂停")
        compare(findChild(queued, "resumeLink").visible, false)
        verify(findChild(queued, "cancelLink").visible)

        var paused = row("84:2")
        compare(findChild(paused, "pauseLink").visible, false)
        verify(findChild(paused, "resumeLink").visible,
               "暂停的上传可以继续")
        verify(findChild(paused, "cancelLink").visible)

        var failed = row("85:0")
        verify(findChild(failed, "retryLink").visible, "失败上传可从头重试")
        compare(findChild(failed, "revealLink").visible, false,
                "上传没有本地落点可打开")

        mouseClick(running, 120, running.height / 2, Qt.RightButton)
        var menu = findChild(page, "transferContextMenu")
        tryVerify(function() { return menu.opened })
        compare(findChild(menu, "transferPauseItem").visible, false)
        compare(findChild(menu, "transferResumeItem").visible, false)
        verify(findChild(menu, "transferCancelItem").visible)
        compare(findChild(menu, "transferRetryItem").visible, false)
        compare(findChild(menu, "transferRevealItem").visible, false)
        menu.close()

        mouseClick(queued, 120, queued.height / 2, Qt.RightButton)
        tryVerify(function() { return menu.opened })
        verify(findChild(menu, "transferPauseItem").visible)
        compare(findChild(menu, "transferResumeItem").visible, false)
        verify(findChild(menu, "transferCancelItem").visible)
        menu.close()

        mouseClick(paused, 120, paused.height / 2, Qt.RightButton)
        tryVerify(function() { return menu.opened })
        compare(findChild(menu, "transferPauseItem").visible, false)
        verify(findChild(menu, "transferResumeItem").visible)
        verify(findChild(menu, "transferCancelItem").visible)
        menu.close()
    }

    function test_partially_failed_folder_retries_child_but_removes_only_parent_record() {
        backend.setDetails("82:folder", [
            makeRow("82:0", "失败子文件.mkv", "failed", {
                detail: "断网"
            })
        ])
        backend.publish([
            makeRow("82:folder", "剧集", "failed", {
                folder: true,
                childCount: 1,
                failedCount: 1
            })
        ], 0, 0)
        waitRows(1)

        mouseClick(findChild(row("82:folder"), "rowHeader"))
        waitRows(2)
        var menu = findChild(page, "transferContextMenu")

        mouseClick(row("82:0"), 120, 24, Qt.RightButton)
        tryVerify(function() { return menu.opened })
        verify(findChild(menu, "transferRetryItem").visible)
        compare(findChild(menu, "transferRemoveItem").visible, false,
                "子行不能只从历史里移除")
        menu.close()

        mouseClick(row("82:folder"), 120, 24, Qt.RightButton)
        tryVerify(function() { return menu.opened })
        compare(findChild(menu, "transferRetryItem").visible, false,
                "部分失败文件夹不能整批重试")
        verify(findChild(menu, "transferRemoveItem").visible,
               "顶层文件夹记录可以整批移除")
    }

    function test_remove_record_requires_confirmation_and_keeps_files_notice() {
        backend.publish([
            makeRow("83:folder", "剧集", "failed", {
                folder: true,
                childCount: 2,
                failedCount: 2
            })
        ], 0, 0)
        waitRows(1)

        mouseClick(row("83:folder"), 120, 24, Qt.RightButton)
        var menu = findChild(page, "transferContextMenu")
        tryVerify(function() { return menu.opened })
        mouseClick(findChild(menu, "transferRemoveItem"))
        wait(40)

        var dialog = findChild(page, "removeRecordConfirmDialog")
        verify(dialog !== null && dialog.opened)
        compare(backend.removeCount, 0, "确认前不能移除记录")
        var message = findChild(dialog, "removeRecordConfirmText").text
        verify(message.indexOf("2 条子记录") >= 0, "文件夹确认框要说明整批子记录")
        verify(message.indexOf("断点都会保留") >= 0, "确认框要说明不会删除断点")

        var confirm = findChild(dialog, "removeRecordConfirmButton")
        verify(confirm !== null)
        compare(confirm.variant, "danger")
        mouseClick(confirm)
        wait(40)
        compare(backend.removeCount, 1)
        compare(backend.removedKey, "83:folder")
    }

    function test_parent_and_child_cancel_actions_are_separate() {
        backend.setDetails("40:folder", [
            makeRow("40:0", "a.mkv", "running", {
                progress: 15,
                progressText: "15%"
            }),
            makeRow("40:1", "b.mkv", "queued")
        ])
        backend.publish([
            makeRow("40:folder", "剧集", "running", {
                folder: true,
                childCount: 2,
                progress: 15,
                progressText: "1/2"
            })
        ], 1, 0)
        waitRows(1)

        mouseClick(findChild(row("40:folder"), "rowHeader"))
        waitRows(3)
        mouseClick(findChild(row("40:0"), "cancelLink"))
        wait(20)
        compare(backend.cancelledKey, "40:0")

        mouseClick(findChild(row("40:folder"), "cancelLink"))
        wait(20)
        compare(backend.cancelledKey, "40:folder")
        compare(backend.cancelCount, 2)
    }

    function test_pause_and_resume_actions_are_top_level_only() {
        backend.publish([
            makeRow("41:0", "下载中.mkv", "running", {
                progress: 25,
                progressText: "25%"
            }),
            makeRow("41:1", "正在暂停.mkv", "pausing", {
                progress: 30,
                progressText: "30%"
            }),
            makeRow("42:folder", "暂停目录", "paused", {
                folder: true,
                childCount: 1,
                progress: 40,
                progressText: "40%"
            })
        ], 1, 0)
        waitRows(3)

        var running = row("41:0")
        verify(findChild(running, "pauseLink").visible)
        compare(findChild(running, "resumeLink").visible, false)
        mouseClick(findChild(running, "pauseLink"))
        wait(20)
        compare(backend.pausedKey, "41:0")
        compare(backend.pauseCount, 1)

        var pausing = row("41:1")
        compare(findChild(pausing, "pauseLink").visible, false)
        compare(findChild(pausing, "resumeLink").visible, false)
        compare(findChild(pausing, "cancelLink").visible, true)

        var paused = row("42:folder")
        verify(findChild(paused, "resumeLink").visible)
        compare(findChild(paused, "pauseLink").visible, false)
        compare(findChild(paused, "cancelLink").visible, true)
        mouseClick(findChild(paused, "resumeLink"))
        wait(20)
        compare(backend.resumedKey, "42:folder")
        compare(backend.resumeCount, 1)
        compare(page.statusText, "1 项传输中 · 1 项已暂停")
    }

    function test_paused_child_has_no_individual_actions() {
        backend.setDetails("43:folder", [
            makeRow("43:0", "子文件.mkv", "paused", {
                progress: 30,
                progressText: "30%"
            })
        ])
        backend.publish([
            makeRow("43:folder", "暂停目录", "paused", {
                folder: true,
                childCount: 1,
                progress: 30,
                progressText: "30%"
            })
        ], 0, 0)
        waitRows(1)
        mouseClick(findChild(row("43:folder"), "rowHeader"))
        waitRows(2)

        var child = row("43:0")
        verify(child !== null)
        compare(findChild(child, "pauseLink").visible, false)
        compare(findChild(child, "resumeLink").visible, false)
        compare(findChild(child, "cancelLink").visible, false)
        compare(findChild(child, "rowProgress").visible, true)
    }

    function test_aggregate_row_uses_parent_live_values() {
        backend.setDetails("50:folder", [
            makeRow("50:0", "a.mkv", "running", {
                progress: 40,
                progressText: "40%",
                sizeText: "4 MB / 10 MB"
            })
        ])
        backend.publish([
            makeRow("50:folder", "剧集", "running", {
                folder: true,
                childCount: 2,
                progress: 20,
                progressText: "1/2",
                sizeText: "2 MB / 10 MB"
            })
        ], 1, 0)
        backend.setLive([
            {
                key: "50:folder",
                progress: 65,
                progressText: "65% · 2/3",
                sizeText: "6.5 MB / 10 MB",
                rateText: "1.2 MB/s",
                noteText: "剩余 3 秒"
            },
            {
                key: "50:0",
                progress: 40,
                progressText: "40%",
                sizeText: "4 MB / 10 MB",
                rateText: "900 KB/s",
                noteText: "剩余 8 秒"
            }
        ])
        waitRows(1)

        var parent = row("50:folder")
        verify(parent !== null)
        compare(findChild(parent, "progressText").text, "65% · 2/3")
        compare(findChild(parent, "sizeText").text, "6.5 MB / 10 MB")
        compare(findChild(parent, "rateText").text, "1.2 MB/s · 剩余 3 秒")
    }

    function test_reorder_diff_keeps_active_rows_first() {
        backend.publish([
            makeRow("1:0", "失败.mkv", "failed", { detail: "断网" }),
            makeRow("2:0", "完成.mkv", "done", { progress: 100 })
        ], 0, 0)
        waitRows(2)

        backend.publish([
            makeRow("3:0", "新下载.mkv", "running", {
                progress: 1,
                progressText: "1%"
            }),
            makeRow("1:0", "失败.mkv", "failed", { detail: "断网" }),
            makeRow("2:0", "完成.mkv", "done", { progress: 100 })
        ], 1, 0)
        waitRows(3)
        compare(row("3:0").y < row("1:0").y, true)
        compare(row("1:0").y < row("2:0").y, true)
    }

    function test_clear_finished_uses_confirmation() {
        backend.publish([
            makeRow("1:0", "完成.mkv", "done", {
                progress: 100,
                path: "/tmp/完成.mkv"
            })
        ], 0, 0)
        waitRows(1)

        var button = findChild(header, "clearFinishedButton")
        verify(button !== null, "清空按钮不存在")
        compare(button.implicitHeight, 34)
        verify(button.visible, "有已结束记录时清空按钮应可见")
        mouseClick(button)
        wait(40)
        var dialog = findChild(page, "clearConfirmDialog")
        verify(dialog !== null && dialog.opened)
        compare(backend.clearCount, 0, "确认前不能清空")

        var footer = findChild(dialog, "clearConfirmBox")
        verify(footer !== null)
        mouseClick(footer.standardButton(Dialog.Ok))
        wait(40)
        compare(backend.clearCount, 1)
    }

    function test_cancel_all_uses_danger_style_and_confirmation() {
        backend.publish([
            makeRow("1:0", "下载中.mkv", "running", {
                progress: 35,
                progressText: "35%"
            })
        ], 1, 0)
        waitRows(1)

        var button = findChild(header, "cancelAllTransfersButton")
        verify(button !== null, "取消全部按钮不存在")
        compare(button.implicitHeight, 34)
        compare(button.variant, "dangerOutline")
        verify(button.enabled, "有进行中任务时取消全部应可用")
        mouseClick(button)
        wait(40)

        var dialog = findChild(page, "cancelAllConfirmDialog")
        verify(dialog !== null && dialog.opened, "取消全部必须先打开确认框")
        compare(backend.cancelAllCount, 0, "确认前不能取消传输")
        compare(findChild(dialog, "cancelAllConfirmText").text.indexOf("断点") >= 0,
                true, "确认框要说明断点会被删除")
        compare(findChild(dialog, "cancelAllConfirmText").text.indexOf("上传") >= 0,
                true, "确认框要说明上传也会停止且不能续传")

        var confirm = findChild(dialog, "cancelAllConfirmButton")
        verify(confirm !== null)
        compare(confirm.variant, "danger")
        mouseClick(confirm)
        wait(40)
        compare(backend.cancelAllCount, 1)
    }

    function test_upload_only_activates_cancel_all_and_status() {
        backend.publish([
            makeRow("2:0", "上传中.mkv", "running", {
                direction: "upload",
                stateText: "上传中"
            })
        ], 0, 0)
        backend.uploadActive = 1
        backend.changed()
        waitRows(1)

        compare(page.activeCount, 1)
        compare(page.statusText, "1 项传输中")
        verify(findChild(header, "cancelAllTransfersButton").enabled,
               "只有上传进行中也必须能取消全部")
    }

    function test_header_retries_all_failed_items() {
        compare(findChild(header, "openDownloadDirButton"), null,
                "传输页标题行不再提供打开下载目录")
        var button = findChild(header, "retryFailedButton")
        verify(button !== null)
        compare(button.implicitHeight, 34)
        compare(button.visible, false, "没有失败项时不显示批量重试")

        backend.publish([
            makeRow("90:0", "失败一.mkv", "failed", { detail: "断网" }),
            makeRow("90:1", "失败二.mkv", "failed", { detail: "超时" })
        ], 0, 0)
        waitRows(2)
        verify(button.visible, "有失败项时显示批量重试")
        verify(button.enabled)
        mouseClick(button)
        wait(20)
        compare(backend.retryFailedCount, 1)
    }
}
