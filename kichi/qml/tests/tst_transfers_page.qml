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
        signal changed()
        function cancelDownloads() {}
        function openDownloadDir() {}
        function clearFinishedTransfers() {}
        function locateTransfer(sourceId) {}
        function isDownloadPending(id) {
            return false
        }
    }

    Component {
        id: pageComponent
        TransfersPage {}
    }

    property var page: null

    function init() {
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

    /// 一条与 Rust `transfers` 快照同形状的记录（字段取自 transfers.rs）。
    function record(key, phase) {
        var running = phase === "running"
        return {
            key: key,
            name: key + ".mkv",
            sourceId: "src-" + key,
            state: phase,
            stateText: phase,
            running: running,
            progress: running ? 10 : 0,
            progressText: running ? "10%" : "",
            sizeText: "1.0 MB",
            detail: phase === "preparing" ? "正在读取目录结构…"
                  : phase === "failed" ? "下载超时：30 秒没有收到数据（已下载 1.0 MB）"
                  : ""
        }
    }

    /// 模拟一次 Rust 侧的结构变化（属性 + revision + changed 信号）。
    ///
    /// ⚠ 必须按 Rust `transfers::ordered()` 的规则排序后再发布：进行中在前
    /// （批号、序号升序），已结束在后（新 → 旧）。少了这一步，一条记录完成时
    /// 在快照里的位置压根不变，就测不出「完成一行导致整段搬移」这类问题。
    function publish(records) {
        backend.transfers = ordered(records)
        backend.transferRevision += 1
        backend.downloadActive = records.filter(function (item) {
            return item.state === "running" || item.state === "queued"
                   || item.state === "preparing"
        }).length
        backend.changed()
    }

    function isActivePhase(phase) {
        return phase === "queued" || phase === "preparing" || phase === "running"
    }

    /// 与 Rust `transfers::ordered()` 同规则。
    function ordered(records) {
        function pair(key) {
            var parts = key.split(":")
            return [Number(parts[0]), Number(parts[1])]
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

    /// 找到页面里 ListView 的 ListModel（组件没有把 id 导出）。
    function rowsModel() {
        function scan(item) {
            if (!item)
                return null
            if (item.model && item.model.get && item.delegate !== undefined)
                return item.model
            var children = item.children || []
            for (var i = 0; i < children.length; i++) {
                var found = scan(children[i])
                if (found)
                    return found
            }
            return null
        }
        return scan(page)
    }

    function dump() {
        var model = rowsModel()
        var out = []
        for (var i = 0; i < model.count; i++)
            out.push(model.get(i).key + ":" + model.get(i).phase)
        return out.join(",")
    }

    /// 找到页面里的 ListView 本体（测滚动位置用）。
    function findList(item) {
        if (!item)
            return null
        if (item.contentY !== undefined && item.positionViewAtIndex !== undefined && item.model)
            return item
        var children = item.children || []
        for (var i = 0; i < children.length; i++) {
            var found = findList(children[i])
            if (found)
                return found
        }
        return null
    }

    // 触发过卡死的原始序列：失败记录留着，重新下载。
    function test_restart_after_failure_keeps_working() {
        publish([record("1:0", "failed")])
        wait(50)
        compare(rowsModel().count, 1)

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
        compare(rowsModel().count, 4)
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
                state: "notice",
                stateText: "",
                running: false,
                progress: 0,
                progressText: "",
                sizeText: "",
                detail: ""
            }
        }
        publish([record("1:0", "failed"), notice(5)])
        wait(20)
        compare(rowsModel().count, 2)
        publish([record("2:0", "running"), record("1:0", "failed"), notice(5)])
        wait(20)
        compare(rowsModel().count, 3)
        compare(rowsModel().get(0).key, "2:0")
        compare(rowsModel().get(2).phase, "notice")
    }

    // 清空后重新开始。
    function test_clear_then_download_again() {
        publish([record("1:0", "failed")])
        wait(20)
        publish([])
        wait(20)
        compare(rowsModel().count, 0)
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

        var list = findList(page)
        verify(list !== null, "找不到 ListView")
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
        compare(rowsModel().count, 35)
    }

    // 定位到某一行仍然要滚过去（这是主动行为，不能被上一条的「别乱滚」误伤）。
    function test_focus_still_scrolls_to_the_row() {
        var records = []
        for (var i = 0; i < 40; i++)
            records.push(record("0:" + i, "done"))
        records.unshift(record("1:0", "queued"))
        publish(records)
        wait(80)
        var list = findList(page)
        list.contentY = 0
        wait(30)

        page.focusSource("src-1:0")
        wait(80)
        compare(page.highlightedKey, "1:0")
        verify(list.contentY < 200, "不该滚到底部，实际 contentY=" + list.contentY)
    }

    // 点已占用的条目：按 sourceId 定位到那一行并高亮。
    function test_focus_source_id_lights_the_right_row() {        publish([record("2:0", "running"), record("1:0", "queued")])
        wait(20)

        page.focusSource("src-1:0")
        wait(60)
        // 定位到的是 1:0 那一行，高亮的也必须是它
        var model = rowsModel()
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
        compare(rowsModel().count, 2)
        compare(rowsModel().get(0).phase, "preparing")
        compare(rowsModel().get(0).sourceId, "src-1:0")
        compare(backend.downloadActive, 2, "准备中也算进行中")
    }
}
