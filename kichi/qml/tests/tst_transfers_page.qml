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
        property var transferLive: []
        signal changed()
        function cancelDownloads() {}
        function openDownloadDir() {}
        function clearFinishedTransfers() {}
    }

    Component {
        id: pageComponent
        TransfersPage {}
    }

    property var page: null

    function init() {
        page = pageComponent.createObject(null, {
            width: 900,
            height: 600,
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
            state: phase,
            stateText: phase,
            running: running,
            progress: running ? 10 : 0,
            progressText: running ? "10%" : "",
            sizeText: "1.0 MB",
            detail: phase === "failed" ? "下载超时：30 秒没有收到数据（已下载 1.0 MB）" : ""
        }
    }

    /// 模拟一次 Rust 侧的结构变化（属性 + revision + changed 信号）。
    function publish(records) {
        backend.transfers = records
        backend.transferRevision += 1
        backend.downloadActive = records.filter(function (item) {
            return item.state === "running" || item.state === "queued"
        }).length
        backend.changed()
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
}
