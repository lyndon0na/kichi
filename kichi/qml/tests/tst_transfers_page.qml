import QtQuick
import QtTest
import ".."

// 传输任务页的列表差异算法回归测试（2026-10-27）。
//
// 为什么要有：`TransfersPage.qml` 的 `syncRows()` 曾在
// 「已有一条失败记录 + 新一批的『下载中』要排到最前面」时死循环
// （`ListModel.move` 目标下标等于 count 会被丢弃，游标不前进），
// 整个界面卡死并刷 `QML ListModel: move: out of range`。
// 症状只在「下载失败/超时后重新下载」时出现，用真实账号复现成本很高，
// 因此把真实组件加载起来、用假后端驱动快照，直接断言行的顺序。
//
// 2026-10-27 加文件夹详情页后：主列表一个批次一行，子行只在
// `TransferFolderPage.qml` 的列表里；两个 ListView 都常驻（StackLayout 保活），
// 所以测试一律按 objectName 取列表，不再扫描「第一个 ListView」。
//
// 运行：
//   QT_QPA_PLATFORM=offscreen /usr/lib64/qt6/bin/qmltestrunner \
//       -input kichi/qml/tests/tst_transfers_page.qml

TestCase {
    name: "TransfersPageRows"
    when: windowShown
    width: 900
    height: 600

    // 替代 Kichi 桥接对象：只提供 TransfersPage 用到的属性 / 信号 / 方法。
    QtObject {
        id: backend
        property var transfers: []
        property int transferRevision: 0
        property int downloadActive: 0
        property int downloadQueued: 0
        property var transferLive: []
        property var transferDetail: []
        property bool transferDetailActive: false

        /// 用例注入的详情快照（模拟 Rust 按选中批次生成的 transferDetail）
        property var detailData: []

        /// 逐条动作的记录：断言「点了哪一行、调了几次」
        property int revealCount: 0
        property string revealedKey: ""
        property int retryCount: 0
        property string retriedKey: ""
        property int openCount: 0
        property string openedKey: ""
        property int closeCount: 0
        /// 由具体用例注入：模拟 Rust 在点击处理期间同步换掉整份记录
        property var onRetry: null

        signal changed()

        function cancelDownloads() {}
        function openDownloadDir() {}
        function clearFinishedTransfers() {}
        function locateTransfer(sourceId) {}
        function isDownloadPending(id) {
            return false
        }
        function revealTransfer(key) {
            revealCount += 1
            revealedKey = key
        }
        function retryTransfer(key) {
            retryCount += 1
            retriedKey = key
            if (onRetry !== null)
                onRetry(key)
        }
        function openTransferFolder(key) {
            openCount += 1
            openedKey = key
            var found = null
            for (var i = 0; i < transfers.length; i++) {
                var row = transfers[i]
                if (row.key === key && row.folder && row.childCount > 0)
                    found = row
            }
            if (found !== null)
                transferDetail = detailData
            transferDetailActive = found !== null
        }
        function closeTransferFolder() {
            closeCount += 1
            transferDetailActive = false
            transferDetail = []
        }
    }

    Component {
        id: pageComponent
        TransfersPage {}
    }

    property var page: null

    function init() {
        backend.transfers = []
        backend.transferRevision = 0
        backend.downloadActive = 0
        backend.downloadQueued = 0
        backend.transferLive = []
        backend.transferDetail = []
        backend.transferDetailActive = false
        backend.detailData = []
        backend.revealCount = 0
        backend.revealedKey = ""
        backend.retryCount = 0
        backend.retriedKey = ""
        backend.openCount = 0
        backend.openedKey = ""
        backend.closeCount = 0
        backend.onRetry = null
        // 挂在测试窗口下并可见：滚动位置相关的断言需要真实的布局几何
        // （`createObject(null)` 建出来的页面没有窗口，contentHeight 恒为 0）
        page = pageComponent.createObject(parent, {
            width: 900,
            height: 600,
            visible: true,
            backend: backend
        })
        verify(page !== null, "TransfersPage 创建失败")
    }

    function cleanup() {
        if (page)
            page.destroy()
        page = null
    }

    // —— 假记录（字段取自 transfers_view.rs 的快照）——

    function stateTextOf(phase) {
        if (phase === "queued")
            return "排队中"
        if (phase === "preparing")
            return "准备中"
        if (phase === "running")
            return "下载中"
        if (phase === "done")
            return "已完成"
        if (phase === "failed")
            return "失败"
        if (phase === "cancelled")
            return "已取消"
        return ""
    }

    /// 一条与 Rust `transfers` 快照同形状的单文件行。
    function record(key, phase) {
        var running = phase === "running"
        return {
            key: key,
            name: key + ".mkv",
            sourceId: "src-" + key,
            state: phase,
            stateText: stateTextOf(phase),
            running: running,
            progress: running ? 10 : 0,
            progressText: running ? "10%" : "",
            sizeText: "1.0 MB",
            detail: phase === "preparing" ? "正在读取目录结构…"
                  : phase === "failed" ? "下载超时：30 秒没有收到数据（已下载 1.0 MB）"
                  : phase === "done" ? "保存到 /home/me/Downloads/" + key + ".mkv"
                  : "",
            path: phase === "done" ? "/home/me/Downloads/" + key + ".mkv" : "",
            folder: false,
            childCount: 0,
            failedCount: 0
        }
    }

    /// 文件夹批次的总账行；选项覆盖聚合字段。
    function folderRecord(key, phase, options) {
        var row = record(key, phase)
        row.name = options && options.name !== undefined
                ? options.name : "文件夹 " + key
        row.folder = true
        row.childCount = options && options.childCount !== undefined
                ? options.childCount : 0
        row.failedCount = options && options.failedCount !== undefined
                ? options.failedCount : 0
        if (options) {
            if (options.stateText !== undefined)
                row.stateText = options.stateText
            if (options.detail !== undefined)
                row.detail = options.detail
            if (options.path !== undefined)
                row.path = options.path
            if (options.running !== undefined)
                row.running = options.running
            if (options.progress !== undefined)
                row.progress = options.progress
            if (options.progressText !== undefined)
                row.progressText = options.progressText
            if (options.sizeText !== undefined)
                row.sizeText = options.sizeText
        }
        return row
    }

    /// 详情页里的一条子文件行。
    function childRecord(key, phase, name) {
        var row = record(key, phase)
        if (name !== undefined)
            row.name = name
        return row
    }

    /// 模拟一次 Rust 侧的结构变化（属性 + revision + changed 信号）。
    ///
    /// ⚠ 必须按 Rust `transfers::ordered()` 的规则排序后再发布：进行中在前
    /// （批号、序号升序），已结束在后（新 → 旧）。少了这一步，一条记录完成时
    /// 在快照里的位置压根不变，就测不出「完成一行导致整段搬移」这类问题。
    function publish(records, detail) {
        backend.transfers = ordered(records)
        if (detail !== undefined)
            backend.detailData = detail
        if (backend.transferDetailActive)
            backend.transferDetail = backend.detailData
        backend.transferRevision += 1
        backend.downloadActive = backend.transfers.filter(function (item) {
            return item.state === "running" || item.state === "queued"
                   || item.state === "preparing"
        }).length
        backend.changed()
    }

    function isActivePhase(phase) {
        return phase === "queued" || phase === "preparing" || phase === "running"
    }

    /// 与 Rust `transfers::ordered()` 同规则（文件夹 key 的 index 按 0 排）。
    function ordered(records) {
        function pair(key) {
            var parts = key.split(":")
            var index = parts[1] === "folder" ? 0 : Number(parts[1])
            return [Number(parts[0]), index]
        }
        var active = [], finished = []
        for (var i = 0; i < records.length; i++) {
            if (isActivePhase(records[i].state))
                active.push(records[i])
            else
                finished.push(records[i])
        }
        active.sort(function (a, b) {
            var ka = pair(a.key), kb = pair(b.key)
            return (ka[0] - kb[0]) || (ka[1] - kb[1])
        })
        finished.sort(function (a, b) {
            var ka = pair(a.key), kb = pair(b.key)
            return (kb[0] - ka[0]) || (kb[1] - ka[1])
        })
        return active.concat(finished)
    }

    // —— 按 objectName 取控件（主列表与详情列表都常驻，不能扫「第一个 ListView」）——

    function mainList() {
        return findChild(page, "transferList")
    }

    function detailList() {
        return findChild(page, "transferDetailList")
    }

    function mainModel() {
        var list = mainList()
        return list ? list.model : null
    }

    function detailModel() {
        var list = detailList()
        return list ? list.model : null
    }

    function dump() {
        var model = mainModel()
        var out = []
        for (var i = 0; i < model.count; i++)
            out.push(model.get(i).key + ":" + model.get(i).phase)
        return out.join(",")
    }

    function dumpDetail() {
        var model = detailModel()
        var out = []
        for (var i = 0; i < model.count; i++)
            out.push(model.get(i).key + ":" + model.get(i).phase)
        return out.join(",")
    }

    /// 第 index 行的 delegate（主列表）。
    function rowAt(index) {
        var list = mainList()
        return list ? list.itemAtIndex(index) : null
    }

    /// 第 index 行的 delegate（详情页）。
    function detailRowAt(index) {
        var list = detailList()
        return list ? list.itemAtIndex(index) : null
    }

    /// 在 delegate 里按 objectName 找控件（contentItem 不一定在 children 里）。
    function findByName(item, name) {
        if (!item)
            return null
        if (item.objectName === name)
            return item
        var candidates = []
        if (item.children)
            candidates = candidates.concat(item.children)
        if (item.contentItem)
            candidates.push(item.contentItem)
        for (var i = 0; i < candidates.length; i++) {
            var found = findByName(candidates[i], name)
            if (found)
                return found
        }
        return null
    }

    /// 主列表某行里某个 objectName 的控件。
    function controlAt(index, name) {
        var delegate = rowAt(index)
        return delegate ? findByName(delegate, name) : null
    }

    /// 详情列表某行里某个 objectName 的控件。
    function detailControlAt(index, name) {
        var delegate = detailRowAt(index)
        return delegate ? findByName(delegate, name) : null
    }

    function openFolderAt(index) {
        var header = controlAt(index, "rowHeader")
        verify(header !== null, "第 " + index + " 行没有 rowHeader")
        mouseClick(header)
        wait(80)
    }

    // —— 原有：主列表差异算法 ——

    // 触发过卡死的原始序列：失败记录留着，重新下载。
    function test_restart_after_failure_keeps_working() {
        publish([record("1:0", "failed")])
        wait(50)
        compare(mainModel().count, 1)

        publish([record("2:0", "running"), record("1:0", "failed")])
        wait(50)
        compare(dump(), "2:0:running,1:0:failed")
    }

    // 连续超时重试：每批都把新的排到最前，必须每次收敛。
    function test_repeated_retries_do_not_freeze() {
        var records = []
        for (var batch = 1; batch <= 4; batch++) {
            records.unshift(record(batch + ":0", "running"))
            publish(records.slice())
            wait(20)
            records[0] = record(batch + ":0", "failed")
            publish(records.slice())
            wait(20)
        }
        compare(mainModel().count, 4)
        compare(dump(), "4:0:failed,3:0:failed,2:0:failed,1:0:failed")
    }

    // 插入 / 删除 / 重排混在一起（一批完成的常见形状）。
    function test_mixed_insert_remove_reorder() {
        publish([
            record("1:0", "done"),
            record("1:1", "done"),
            record("1:2", "done")
        ])
        wait(20)
        publish([
            record("2:0", "running"),
            record("1:2", "done"),
            record("1:0", "done")
        ])
        wait(20)
        compare(dump(), "2:0:running,1:2:done,1:0:done")

        publish([record("2:0", "cancelled")])
        wait(20)
        compare(dump(), "2:0:cancelled")
    }

    // 记录超过上限时，Rust 会追加一条「还有 N 条未显示」的提示行。
    // 它也必须带 key（Rust 侧固定为 "notice"），否则 ListModel 报
    // `key is undefined`，而且每次刷新都会被当成新行重复插入。
    function test_notice_row_stays_once() {
        function notice(hidden) {
            return {
                key: "notice",
                name: "还有 " + hidden + " 条记录未显示（页面只显示前 500 条）",
                sourceId: "",
                state: "notice",
                stateText: "",
                running: false,
                progress: 0,
                progressText: "",
                sizeText: "",
                detail: "",
                path: "",
                folder: false,
                childCount: 0,
                failedCount: 0
            }
        }
        publish([record("1:0", "failed"), notice(5)])
        wait(20)
        compare(mainModel().count, 2)
        publish([record("2:0", "running"), record("1:0", "failed"), notice(5)])
        wait(20)
        compare(mainModel().count, 3)
        compare(mainModel().get(0).key, "2:0")
        compare(mainModel().get(2).phase, "notice")
    }

    // 清空后重新开始。
    function test_clear_then_download_again() {
        publish([record("1:0", "failed")])
        wait(20)
        publish([])
        wait(20)
        compare(mainModel().count, 0)
        publish([record("2:0", "running")])
        wait(20)
        compare(dump(), "2:0:running")
    }

    // 2026-10-27 用户实测：滚到「看不到正在下载那条」的位置（已完成段中间），
    // 每下完一个文件列表就往下跳一次。根因是 syncRows 正向对齐：一条记录完成时
    // 要从「进行中」段挪到「已完成」段，正向扫描会把它一格一格往后推，
    // 一次完成 = 几十次 move，ListView 内部位置被打乱后被强制重排。
    function test_completion_while_scrolled_away_keeps_scroll() {
        var records = []
        for (var b = 0; b < 30; b++)
            records.push(record("1:" + b, b === 0 ? "running" : "queued"))
        for (var i = 0; i < 5; i++)
            records.push(record("0:" + i, "done"))
        publish(records)
        wait(100)

        var list = mainList()
        verify(list !== null, "找不到主列表 ListView")
        list.contentY = 700
        wait(50)
        var before = list.contentY
        verify(before > 0, "先真的滚下去（contentY=" + before + "）")

        // 逐个完成：每次都要停在原位
        for (var n = 0; n < 30; n++) {
            records[n] = record("1:" + n, "done")
            if (n + 1 < 30)
                records[n + 1] = record("1:" + (n + 1), "running")
            publish(records)
            wait(40)
            if (Math.abs(list.contentY - before) > 1) {
                fail("下完第 " + n + " 个后列表跳了：contentY " + before + " → " + list.contentY)
                return
            }
        }
        compare(mainModel().count, 35)
    }

    // 定位到某一行仍然要滚过去（这是主动行为，不能被上一条的「别乱滚」误伤）。
    function test_focus_still_scrolls_to_the_row() {
        var records = []
        for (var i = 0; i < 40; i++)
            records.push(record("0:" + i, "done"))
        records.unshift(record("1:0", "queued"))
        publish(records)
        wait(80)
        var list = mainList()
        list.contentY = 0
        wait(30)

        page.focusSource("src-1:0")
        wait(80)
        compare(page.highlightedKey, "1:0")
        verify(list.contentY < 200, "不该滚到底部，实际 contentY=" + list.contentY)
    }

    // 点已占用的条目：按 sourceId 定位到那一行并高亮。
    function test_focus_source_id_lights_the_right_row() {
        publish([record("2:0", "running"), record("1:0", "queued")])
        wait(20)

        page.focusSource("src-1:0")
        wait(60)
        // 定位到的是 1:0 那一行，高亮的也必须是它
        var model = mainModel()
        var expectedKey = null
        for (var i = 0; i < model.count; i++) {
            if (model.get(i).sourceId === "src-1:0")
                expectedKey = model.get(i).key
        }
        compare(expectedKey, "1:0")
        compare(page.highlightedKey, "1:0", "高亮的是定位到的那一行")

        // 同一 id 再点一次也要重新定位（不能因为属性值没变就不触发）
        page.highlightedKey = ""
        page.focusSourceId = ""
        page.focusSource("src-1:0")
        wait(60)
        compare(page.highlightedKey, "1:0")
    }

    // 高亮约 1.2 秒后自动淡出。
    function test_highlight_fades_out() {
        publish([record("1:0", "running")])
        wait(20)
        page.focusSource("src-1:0")
        wait(60)
        compare(page.highlightedKey, "1:0")
        wait(1400)
        compare(page.highlightedKey, "", "高亮必须自己消失")
    }

    // 准备中的行渲染不报 QWARN（那也直接判测试失败），并显示说明文案。
    function test_preparing_row_renders() {
        var row = record("1:0", "preparing")
        row.name = "大文件夹"
        row.sizeText = "—"
        publish([row, record("2:0", "queued")])
        wait(60)
        compare(mainModel().count, 2)
        compare(mainModel().get(0).phase, "preparing")
        compare(mainModel().get(0).sourceId, "src-1:0")
        compare(backend.downloadActive, 2, "准备中也算进行中")
    }

    // —— 文件夹批次：主列表只渲染总账行 ——

    function test_folder_batch_is_one_row_in_main_list() {
        var folder = folderRecord("1:folder", "running", {
            childCount: 3,
            running: true,
            progress: 30,
            progressText: "30% · 1/3",
            detail: "第 1/3 个"
        })
        publish([
            folder,
            record("2:0", "done")
        ], [
            childRecord("1:0", "done", "a.mkv"),
            childRecord("1:1", "running", "b.mkv"),
            childRecord("1:2", "queued", "c.mkv")
        ])
        wait(50)

        compare(mainModel().count, 2, "一个文件夹 + 一个单文件 = 两行")
        compare(dump(), "1:folder:running,2:0:done")
        compare(mainModel().get(0).childCount, 3)
        compare(mainModel().get(0).folder, true)
    }

    function test_folder_row_with_children_is_clickable() {
        var folder = folderRecord("1:folder", "running", {
            childCount: 2,
            running: true
        })
        var detail = [
            childRecord("1:0", "done", "a.mkv"),
            childRecord("1:1", "running", "b.mkv")
        ]
        publish([folder], detail)
        wait(50)

        var hint = controlAt(0, "openDetailsHint")
        verify(hint !== null && hint.visible, "有子文件的文件夹行显示 ›")

        openFolderAt(0)
        compare(backend.openCount, 1)
        compare(backend.openedKey, "1:folder")
        compare(page.detailActive, true, "后端激活详情")
        verify(detailList() !== null, "找不到详情页列表")
        compare(detailModel().count, 2)
        compare(dumpDetail(), "1:0:done,1:1:running")
    }

    function test_folder_row_without_children_is_not_clickable() {
        publish([folderRecord("1:folder", "preparing", {
            childCount: 0
        })])
        wait(50)

        var hint = controlAt(0, "openDetailsHint")
        verify(hint === null || !hint.visible, "没有子文件时不显示 ›")
        var header = controlAt(0, "rowHeader")
        verify(header !== null && !header.enabled, "没有子文件时第一行不可点")
        compare(backend.openCount, 0)
    }

    // 返回后主列表的滚动位置不能被重建冲掉。
    function test_back_keeps_main_list_scroll() {
        // 文件夹批次号最小 → 已结束段排最后；滚到底部正好能看到它。
        var records = [
            folderRecord("0:folder", "done", {
                childCount: 2,
                path: "/home/me/Downloads/文件夹"
            })
        ]
        for (var i = 1; i <= 12; i++)
            records.push(record(i + ":0", "done"))
        publish(records, [
            childRecord("0:0", "done", "a.mkv"),
            childRecord("0:1", "done", "b.mkv")
        ])
        wait(100)

        var list = mainList()
        list.contentY = list.contentHeight - list.height
        wait(50)
        var before = list.contentY
        verify(before > 0, "先滚到底部（contentY=" + before + "）")

        var folderIndex = mainModel().count - 1
        openFolderAt(folderIndex)
        compare(page.detailActive, true)

        var back = findChild(page, "backLink")
        verify(back !== null, "详情页找不到返回链接")
        mouseClick(back)
        wait(80)

        compare(page.detailActive, false)
        compare(backend.closeCount, 1)
        compare(mainList().contentY, before, "返回后主列表滚动位置不变")
    }

    // 详情页里完成一个文件：状态变了、顺序不变、滚动位置不动。
    function test_detail_completion_keeps_scroll_and_order() {
        var detail = []
        for (var i = 0; i < 40; i++) {
            detail.push(childRecord("1:" + i, i === 0 ? "running" : "queued",
                                    "file" + i + ".mkv"))
        }
        publish([folderRecord("1:folder", "running", {
            childCount: 40,
            running: true,
            progress: 5,
            progressText: "5% · 1/40",
            detail: "第 1/40 个"
        })], detail)
        wait(100)

        openFolderAt(0)
        var list = detailList()
        verify(list !== null, "找不到详情页列表")
        list.contentY = 300
        wait(50)
        var before = list.contentY
        verify(before > 0, "详情页先滚下去（contentY=" + before + "）")

        var updated = detail.slice()
        updated[0] = childRecord("1:0", "done", "file0.mkv")
        updated[1] = childRecord("1:1", "running", "file1.mkv")
        publish([folderRecord("1:folder", "running", {
            childCount: 40,
            running: true,
            progress: 8,
            progressText: "8% · 2/40",
            detail: "第 2/40 个"
        })], updated)
        wait(120)

        compare(detailModel().count, 40)
        compare(detailModel().get(0).key, "1:0", "完成的文件不移动")
        compare(detailModel().get(0).phase, "done")
        compare(detailList().contentY, before, "详情页滚动位置不变")
    }

    // 详情页子行的动作必须使用子行自己的 key，而不是父行的 key。
    function test_detail_row_actions_use_child_key() {
        var folder = folderRecord("1:folder", "failed", {
            childCount: 2,
            failedCount: 1,
            stateText: "部分失败",
            detail: "成功 1 个 · 失败 1 个"
        })
        publish([folder], [
            childRecord("1:0", "done", "a.mkv"),
            childRecord("1:1", "failed", "b.mkv")
        ])
        wait(60)

        openFolderAt(0)
        compare(detailModel().count, 2)

        var reveal = detailControlAt(0, "revealLink")
        verify(reveal !== null && reveal.visible, "已完成的子行有「打开位置」")
        mouseClick(reveal)
        compare(backend.revealCount, 1)
        compare(backend.revealedKey, "1:0")

        var retry = detailControlAt(1, "retryLink")
        verify(retry !== null && retry.visible, "失败的子行有「重试」")
        mouseClick(retry)
        compare(backend.retryCount, 1)
        compare(backend.retriedKey, "1:1", "重试的是子行自己的 key")

        var hint = detailControlAt(0, "openDetailsHint")
        verify(hint === null || !hint.visible, "子文件行不显示下钻提示")
    }

    // 部分失败：父行显示「部分失败 / 点开可重试」，没有整批重试。
    function test_partial_failure_parent_has_no_retry() {
        publish([folderRecord("1:folder", "failed", {
            childCount: 3,
            failedCount: 2,
            stateText: "部分失败",
            detail: "成功 1 个 · 失败 2 个"
        })], [
            childRecord("1:0", "done", "a.mkv"),
            childRecord("1:1", "failed", "b.mkv"),
            childRecord("1:2", "failed", "c.mkv")
        ])
        wait(60)

        var retry = controlAt(0, "retryLink")
        verify(retry === null || !retry.visible, "父行不提供整批重试")
        var detail = controlAt(0, "detailLink")
        verify(detail !== null, "部分失败要有说明")
        verify(detail.text.indexOf("部分失败") < 0, "状态文字在状态列，不拼进说明")
        verify(detail.text.indexOf("点开可重试") >= 0, "说明行追加下钻提示")
        var hint = controlAt(0, "openDetailsHint")
        verify(hint !== null && hint.visible, "部分失败仍可进详情")
    }

    // 读目录前整体失败：没有子行，父行可整批重试。
    function test_failed_folder_without_children_offers_retry() {        publish([folderRecord("1:folder", "failed", {
            childCount: 0,
            stateText: "失败",
            detail: "目录读取失败"
        })])
        wait(60)

        var hint = controlAt(0, "openDetailsHint")
        verify(hint === null || !hint.visible, "无子行不显示下钻提示")
        var retry = controlAt(0, "retryLink")
        verify(retry !== null && retry.visible, "无子行失败给整批重试")
        mouseClick(retry)
        compare(backend.retryCount, 1)
        compare(backend.retriedKey, "1:folder", "重试的是文件夹总账行")
    }

    // 空文件夹：Done 且没有子行，但有落盘路径（ADR 0012）。
    // 打开位置可用、不显示下钻箭头、不给重试。
    function test_empty_folder_done_row_reveals_its_directory() {
        publish([folderRecord("1:folder", "done", {
            childCount: 0,
            stateText: "已完成",
            sizeText: "—",
            detail: "空文件夹 · 保存到 /home/me/Downloads/空文件夹",
            path: "/home/me/Downloads/空文件夹"
        })])
        wait(60)

        var hint = controlAt(0, "openDetailsHint")
        verify(hint === null || !hint.visible, "空文件夹没有子行，不显示 ›")
        var header = controlAt(0, "rowHeader")
        verify(header !== null && !header.enabled, "空文件夹第一行不可点开详情")

        var reveal = controlAt(0, "revealLink")
        verify(reveal !== null && reveal.visible, "空文件夹可「打开位置」")
        mouseClick(reveal)
        compare(backend.revealCount, 1)
        compare(backend.revealedKey, "1:folder")

        var retry = controlAt(0, "retryLink")
        verify(retry === null || !retry.visible, "空文件夹算完成，不给重试")
        compare(controlAt(0, "stateText").text, "已完成")
        compare(controlAt(0, "sizeText").text, "—")

        var detail = controlAt(0, "detailLink")
        verify(detail !== null, "空文件夹要有说明行")
        compare(detail.text, "空文件夹 · 保存到 /home/me/Downloads/空文件夹")
    }

    // 带跳过数的完成行：说明行原样显示 Rust 拼好的文案。
    function test_done_row_shows_skipped_directories() {
        publish([folderRecord("1:folder", "done", {
            childCount: 3,
            detail: "3 个文件 · 保存到 /home/me/Downloads/剧集 · 跳过 2 个目录",
            path: "/home/me/Downloads/剧集"
        })])
        wait(60)

        var detail = controlAt(0, "detailLink")
        verify(detail !== null)
        verify(detail.text.indexOf("跳过 2 个目录") >= 0,
               "跳过后缀要显示出来：" + detail.text)
        var reveal = controlAt(0, "revealLink")
        verify(reveal !== null && reveal.visible, "有文件落盘的完成行仍可打开位置")
    }

    // 详情页开着时定位别的条目：先回主列表，再滚到目标行。
    function test_focus_source_closes_detail_before_locating() {
        var records = [
            folderRecord("1:folder", "running", {
                childCount: 2,
                running: true
            }),
            record("2:0", "queued")
        ]
        publish(records, [
            childRecord("1:0", "running", "a.mkv"),
            childRecord("1:1", "queued", "b.mkv")
        ])
        wait(60)
        openFolderAt(0)
        compare(page.detailActive, true)

        page.focusSource("src-2:0")
        wait(120)
        compare(page.detailActive, false, "定位前先关详情")
        compare(backend.closeCount, 1)
        compare(page.highlightedKey, "2:0")
    }

    // 实时进度按 key 贴到行上：主列表的文件夹父行与详情页的子行各取各的。
    function test_live_progress_reaches_folder_and_detail_rows() {
        var folder = folderRecord("1:folder", "running", {
            childCount: 50,
            running: true,
            progress: 0,
            progressText: "0% · 1/50",
            sizeText: "0 B / 100 MB"
        })
        var detail = []
        for (var i = 0; i < 50; i++) {
            detail.push(childRecord("1:" + i, i === 0 ? "running" : "queued",
                                    "f" + i + ".mkv"))
        }
        publish([folder], detail)
        wait(80)

        backend.transferLive = [
            { key: "1:folder", progress: 62, progressText: "62% · 31/50",
              sizeText: "62 MB / 100 MB", rateText: "3.7 MB/s · 剩 2 分 30 秒",
              noteText: "" },
            { key: "1:0", progress: 42, progressText: "42%",
              sizeText: "4 MB / 10 MB", rateText: "812 KB/s · 剩 45 秒",
              noteText: "" }
        ]
        wait(80)

        var bar = controlAt(0, "rowProgress")
        verify(bar !== null && bar.visible, "文件夹行显示聚合进度条")
        compare(Math.round(bar.value), 62, "父行取 transferLive 里的聚合进度")
        compare(controlAt(0, "sizeText").text, "62 MB / 100 MB")

        openFolderAt(0)
        wait(80)
        var childBar = detailControlAt(0, "rowProgress")
        verify(childBar !== null && childBar.visible, "详情页当前文件显示进度条")
        compare(Math.round(childBar.value), 42, "详情页子行按自己的 key 取进度")
        compare(detailControlAt(0, "sizeText").text, "4 MB / 10 MB")
    }

    // 批内并发后可能同时有多条「下载中」：每一行都要按自己的 key 取实时进度。
    function test_two_running_files_keep_their_own_live_progress() {
        publish([
            record("1:0", "running"),
            record("1:1", "running")
        ])
        wait(80)

        backend.transferLive = [
            { key: "1:0", progress: 20, progressText: "20%",
              sizeText: "2 MB / 10 MB", rateText: "1 MB/s", noteText: "" },
            { key: "1:1", progress: 70, progressText: "70%",
              sizeText: "7 MB / 10 MB", rateText: "2 MB/s", noteText: "" }
        ]
        wait(80)

        compare(Math.round(controlAt(0, "rowProgress").value), 20,
                "第一行只吃自己的进度")
        compare(Math.round(controlAt(1, "rowProgress").value), 70,
                "第二行也要实时刷新，不能停在开始时的值")
        compare(controlAt(0, "sizeText").text, "2 MB / 10 MB")
        compare(controlAt(1, "sizeText").text, "7 MB / 10 MB")
        compare(controlAt(0, "rateText").text, "1 MB/s")
        compare(controlAt(1, "rateText").text, "2 MB/s")
    }

    // —— 速度 / 剩余时间 / 卡住 / 重试说明（2026-10-27）——

    // 主列表文件夹父行显示整批速度 + ETA；详情页头部也显示整批速度。
    function test_folder_row_and_detail_header_show_batch_rate() {
        var folder = folderRecord("1:folder", "running", {
            childCount: 50,
            running: true,
            progress: 62,
            progressText: "62% · 31/50",
            sizeText: "1.2 GB / 3.4 GB"
        })
        publish([folder], [
            childRecord("1:0", "done", "a.mkv"),
            childRecord("1:1", "running", "b.mkv"),
            childRecord("1:2", "queued", "c.mkv")
        ])
        wait(80)

        backend.transferLive = [
            { key: "1:folder", progress: 62, progressText: "62% · 31/50",
              sizeText: "1.2 GB / 3.4 GB",
              rateText: "3.7 MB/s · 剩 2 分 30 秒", noteText: "" },
            { key: "1:1", progress: 42, progressText: "42%",
              sizeText: "4 MB / 10 MB",
              rateText: "3.7 MB/s · 剩 1 分 30 秒", noteText: "" }
        ]
        wait(80)

        var rate = controlAt(0, "rateText")
        verify(rate !== null && rate.visible, "下载中的文件夹行速度列可见")
        compare(rate.text, "3.7 MB/s · 剩 2 分 30 秒", "父行显示整批速度 + ETA")
        compare(controlAt(0, "openDetailsHint").visible, true, "运行中的文件夹行保留下钻提示")

        openFolderAt(0)
        wait(80)
        var header = findChild(page, "folderRateText")
        verify(header !== null && header.visible, "详情页头部显示整批速度")
        compare(header.text, "3.7 MB/s · 剩 2 分 30 秒")

        // 详情页子行按自己的 key 取单文件速度
        var childRate = detailControlAt(1, "rateText")
        verify(childRate !== null && childRate.visible, "详情页当前文件显示速度列")
        compare(childRate.text, "3.7 MB/s · 剩 1 分 30 秒")
        var finishedRate = detailControlAt(0, "rateText")
        verify(finishedRate === null || !finishedRate.visible,
               "已完成的子行不显示速度")
    }

    // 卡住文案替换速度；百分比、进度条、状态文字不变。
    function test_stall_text_replaces_speed_without_touching_progress() {
        publish([record("1:0", "running")])
        wait(60)
        var stateLabel = controlAt(0, "stateText")

        backend.transferLive = [
            { key: "1:0", progress: 41, progressText: "41%",
              sizeText: "4 MB / 10 MB",
              rateText: "已卡住 7 秒", noteText: "" }
        ]
        wait(60)

        var rate = controlAt(0, "rateText")
        verify(rate !== null && rate.visible)
        compare(rate.text, "已卡住 7 秒", "卡住时只报卡住")
        compare(controlAt(0, "rowProgress").value, 41, "进度条不动")
        compare(controlAt(0, "sizeText").text, "4 MB / 10 MB", "大小列不动")
        compare(controlAt(0, "stateText").text, "下载中", "状态列不变色、不改字")
        verify(controlAt(0, "stateText") === stateLabel, "状态文字还是同一个控件")

        // 恢复后切回速度
        backend.transferLive = [
            { key: "1:0", progress: 43, progressText: "43%",
              sizeText: "4.3 MB / 10 MB",
              rateText: "3.7 MB/s · 剩 2 秒", noteText: "" }
        ]
        wait(60)
        compare(controlAt(0, "rateText").text, "3.7 MB/s · 剩 2 秒")
    }

    // 重试说明出现在第二行；进度条仍然可见；说明为空时占位高度不变。
    function test_note_appears_below_and_keeps_the_progress_bar() {
        publish([folderRecord("1:folder", "running", {
            childCount: 3,
            running: true,
            progressText: "41% · 12/30"
        })], [
            childRecord("1:0", "running", "a.mkv")
        ])
        wait(60)

        backend.transferLive = [
            { key: "1:folder", progress: 41, progressText: "41% · 12/30",
              sizeText: "840 MB / 2.1 GB",
              rateText: "已卡住 7 秒",
              noteText: "3 秒后从 45 MB 继续（1/3）" },
            { key: "1:0", progress: 41, progressText: "41%",
              sizeText: "840 MB / 2.1 GB",
              rateText: "已卡住 7 秒",
              noteText: "3 秒后从 45 MB 继续（1/3）" }
        ]
        wait(60)

        var note = controlAt(0, "noteText")
        verify(note !== null && note.visible, "下载中的行显示重试说明")
        compare(note.text, "3 秒后从 45 MB 继续（1/3）")
        var bar = controlAt(0, "rowProgress")
        verify(bar !== null && bar.visible, "说明出现时进度条仍在")
        compare(controlAt(0, "rateText").text, "已卡住 7 秒", "卡住与重试说明可以共存")

        // 说明过期（Rust 给空串）：标签消失、行高不变
        var heightBefore = rowAt(0).height
        backend.transferLive = [
            { key: "1:folder", progress: 45, progressText: "45% · 13/30",
              sizeText: "900 MB / 2.1 GB",
              rateText: "3.7 MB/s · 剩 5 分", noteText: "" },
            { key: "1:0", progress: 45, progressText: "45%",
              sizeText: "900 MB / 2.1 GB",
              rateText: "3.7 MB/s · 剩 1 分", noteText: "" }
        ]
        wait(60)
        var gone = controlAt(0, "noteText")
        verify(gone === null || !gone.visible, "说明过期后消失")
        compare(rowAt(0).height, heightBefore, "第二行槽位固定，行高不变")
    }

    // 排队 / 完成 / 失败行：不显示速度、卡住、说明（说明仍走原 detail）。
    function test_idle_rows_have_no_rate_or_note() {
        // 进行中在前；已结束新→旧（3:0 失败、2:0 完成）
        publish([
            record("1:0", "queued"),
            record("2:0", "done"),
            record("3:0", "failed")
        ])
        wait(60)

        for (var i = 0; i < 3; i++) {
            var rate = controlAt(i, "rateText")
            verify(rate === null || !rate.visible,
                   "第 " + i + " 行不该有速度列")
            var note = controlAt(i, "noteText")
            verify(note === null || !note.visible,
                   "第 " + i + " 行不该有下载说明")
        }
        // 失败原因仍按原路径显示在说明行
        var detail = controlAt(1, "detailLink")
        verify(detail !== null && detail.text.indexOf("下载超时") >= 0,
               "失败原因照旧")
    }

    // —— 单条动作：完成行「打开位置」/ 失败行「重试」（2026-10-27）——

    // 完成行：整段路径与右端链接调同一个方法，key 必须是这一行。
    function test_done_row_path_and_link_reveal_the_file() {
        publish([record("1:0", "done")])
        wait(50)

        var detail = controlAt(0, "detailLink")
        verify(detail !== null, "完成行要能找到路径说明")
        verify(detail.text.indexOf("保存到") === 0, "路径仍是说明文字")
        mouseClick(detail)
        compare(backend.revealCount, 1, "点整段路径调一次 reveal")
        compare(backend.revealedKey, "1:0")

        var link = controlAt(0, "revealLink")
        verify(link !== null && link.visible, "完成行显示「打开位置」")
        mouseClick(link)
        compare(backend.revealCount, 2, "点「打开位置」也调 reveal")
        compare(backend.revealedKey, "1:0")

        var retry = controlAt(0, "retryLink")
        verify(retry === null || !retry.visible, "完成行不给重试")
    }

    // 失败行：只显示「重试」，路径不可点。
    function test_failed_row_retries() {
        publish([record("1:0", "failed")])
        wait(50)

        var reveal = controlAt(0, "revealLink")
        verify(reveal === null || !reveal.visible, "失败行没有「打开位置」")

        var detail = controlAt(0, "detailLink")
        verify(detail !== null, "失败原因照常显示")
        mouseClick(detail)
        compare(backend.revealCount, 0, "失败行点路径不该去定位文件")

        var retry = controlAt(0, "retryLink")
        verify(retry !== null && retry.visible, "失败行显示「重试」")
        mouseClick(retry)
        compare(backend.retryCount, 1)
        compare(backend.retriedKey, "1:0", "重试的是这一行")
    }

    // 排队 / 下载中 / 已取消：两个行内动作都不出现。
    function test_only_finished_or_failed_rows_have_actions() {
        publish([
            record("1:0", "queued"),
            record("2:0", "running"),
            record("3:0", "cancelled")
        ])
        wait(50)
        for (var i = 0; i < 3; i++) {
            var reveal = controlAt(i, "revealLink")
            var retry = controlAt(i, "retryLink")
            verify(reveal === null || !reveal.visible,
                   "第 " + i + " 行不该有「打开位置」")
            verify(retry === null || !retry.visible, "第 " + i + " 行不该有「重试」")
        }
        compare(backend.revealCount, 0)
        compare(backend.retryCount, 0)
    }

    // 点击「重试」时 Rust 会同步删掉旧失败行、加入新排队行——被点的 delegate
    // 当场销毁。必须零 QWARN（qml-lint.sh 把 QWARN 当失败）。
    function test_retry_click_can_delete_its_own_row() {
        publish([record("1:0", "failed")])
        wait(50)
        backend.onRetry = function (key) {
            // 与 Rust 一样：删旧行 + 加新排队行，同步发快照
            publish([record("2:0", "queued")])
        }

        var retry = controlAt(0, "retryLink")
        verify(retry !== null && retry.visible)
        mouseClick(retry)

        compare(backend.retryCount, 1)
        compare(backend.retriedKey, "1:0")
        wait(50)
        compare(dump(), "2:0:queued", "旧失败行消失，新排队行出现")
        compare(mainModel().count, 1)
    }
}
