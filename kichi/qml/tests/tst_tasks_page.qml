import QtQuick
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

        signal changed()

        property int submitted: 0
        property string deletedId: ""

        function addTask(url) {
            submitted += 1
        }
        function deleteTask(id) {
            deletedId = id
        }
        function refreshTasks() {}
    }

    TasksPage {
        id: page
        backend: backend
        width: 800
        height: 600
    }

    // 主列表（TasksPage 里只有一个 ListView，按 objectName 取更稳）
    property var list: findChild(page, "tasksList")
    property var model: list ? list.model : null

    /// 造一条与 Rust `task_map` 同形状的任务
    function task(id, phase, progress) {
        var settled = phase === "completed" || phase === "error"
        return {
            id: id,
            name: id + ".mkv",
            phase: phase,
            phaseText: phase,
            progress: progress,
            progressText: progress + "%",
            sizeText: "1 GB",
            createdText: "10-28 12:00",
            detail: "",
            settled: settled
        }
    }

    function push(tasks) {
        backend.tasks = tasks
        backend.changed()
    }

    function test_rows_follow_backend_order() {
        push([task("t1", "running", 10), task("t2", "completed", 100)])
        compare(model.count, 2)
        compare(model.get(0).key, "t1")
        compare(model.get(1).key, "t2")
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
}
