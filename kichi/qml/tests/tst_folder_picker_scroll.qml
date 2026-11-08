import QtQuick
import QtTest
import "../dialogs"
import "../tasks"

// 两个目录选择器的导航回归测试：
// 滚轮后首击直接进入当前可见目录；进入子目录时在当前位置原子替换内容。
TestCase {
    name: "FolderPickerScroll"
    when: windowShown
    width: 1000
    height: 800

    QtObject {
        id: taskBackend

        property var moveTargets: []
        property int moveTargetsRevision: 0
        property bool moveTargetsLoading: false
        property string moveTargetsError: ""
        property bool delayedLoads: false
        /// 子目录条目数少一档：用来复现「长目录底部进入短目录」
        property bool shortChildren: false
        property var pendingItems: []

        signal changed()

        function currentPath() {
            return [{ id: "", name: "全部文件" }]
        }

        function loadMoveTargets(parentId) {
            const items = []
            if (parentId === "") {
                for (let i = 0; i < 30; i++) {
                    items.push({
                        id: "task-folder-" + i,
                        name: "任务文件夹 " + i
                    })
                }
            } else {
                const count = shortChildren ? 3 : 30
                for (let i = 0; i < count; i++) {
                    items.push({
                        id: parentId + "-child-" + i,
                        name: "任务子目录 " + i
                    })
                }
            }
            pendingItems = items
            moveTargetsLoading = true
            moveTargetsError = ""
            changed()
            if (delayedLoads)
                taskLoadTimer.restart()
            else
                finishDelayedLoad()
        }

        function finishDelayedLoad() {
            if (!moveTargetsLoading)
                return
            moveTargets = pendingItems
            pendingItems = []
            moveTargetsLoading = false
            moveTargetsRevision += 1
            changed()
        }

        function cancelMoveTargets() {
        }
    }

    Timer {
        id: taskLoadTimer
        interval: 60
        repeat: false
        onTriggered: taskBackend.finishDelayedLoad()
    }

    QtObject {
        id: moveBackend

        property var moveTargets: []
        property int moveTargetsRevision: 0
        property bool moveTargetsLoading: false
        property string moveTargetsError: ""
        property bool fileActionRunning: false
        property var fileActionIds: []
        property bool shortChildren: false
        property var current: [{ id: "", name: "全部文件" }]

        signal changed()
        signal fileActionDone(string action, bool ok, string message, var ids)

        function currentPath() {
            return current
        }

        function loadMoveTargets(parentId) {
            const items = []
            if (parentId === "") {
                for (let i = 0; i < 30; i++) {
                    items.push({
                        id: "move-folder-" + i,
                        name: "移动文件夹 " + i
                    })
                }
            } else {
                const count = shortChildren ? 3 : 30
                for (let i = 0; i < count; i++) {
                    items.push({
                        id: parentId + "-child-" + i,
                        name: "移动子目录 " + i
                    })
                }
            }
            moveTargets = items
            moveTargetsLoading = false
            moveTargetsError = ""
            moveTargetsRevision += 1
            changed()
        }

        function cancelMoveTargets() {
        }
    }

    Component {
        id: taskPickerComponent
        TaskFolderPickerDialog {}
    }

    Component {
        id: movePickerComponent
        MovePickerDialog {}
    }

    property var taskPicker: null
    property var movePicker: null

    function init() {
        taskBackend.moveTargets = []
        taskBackend.moveTargetsRevision = 0
        taskBackend.moveTargetsLoading = false
        taskBackend.moveTargetsError = ""
        taskBackend.delayedLoads = false
        taskBackend.shortChildren = false
        taskBackend.pendingItems = []
        taskLoadTimer.stop()

        moveBackend.moveTargets = []
        moveBackend.moveTargetsRevision = 0
        moveBackend.moveTargetsLoading = false
        moveBackend.moveTargetsError = ""
        moveBackend.fileActionRunning = false
        moveBackend.fileActionIds = []
        moveBackend.shortChildren = false
        moveBackend.current = [{ id: "", name: "全部文件" }]
    }

    function cleanup() {
        if (taskPicker)
            taskPicker.destroy()
        if (movePicker)
            movePicker.destroy()
        taskPicker = null
        movePicker = null
    }

    function openTaskPicker() {
        taskPicker = taskPickerComponent.createObject(parent, {
            backend: taskBackend
        })
        verify(taskPicker)
        taskPicker.openPicker([])
        wait(60)
        const list = findChild(taskPicker, "taskFolderPickerList")
        verify(list)
        verify(list.contentHeight > list.height)
        return list
    }

    function openMovePicker() {
        movePicker = movePickerComponent.createObject(parent, {
            backend: moveBackend
        })
        verify(movePicker)
        movePicker.openForMove({
            id: "source-file",
            name: "源文件",
            kind: "file"
        }, ["source-file"], [])
        wait(60)
        const list = findChild(movePicker, "movePickerList")
        verify(list)
        verify(list.contentHeight > list.height)
        return list
    }

    // 惯性滚动中 ListView 可能有一帧拿不到中心点下的 delegate（或指针正好落在
    // 两行之间的 spacing 里），等坐标重新落到某一行再继续，避免把渲染计时差异
    // 当成「首击失效」。
    function centerIndex(list) {
        const x = list.width / 2
        const y = list.height / 2
        let index = -1
        tryVerify(function () {
            index = list.indexAt(x, y + list.contentY)
            return index >= 0
        }, 1000, "滚轮后列表中心必须有可点击的行")
        return index
    }

    function wheelThenClickFirstVisible(list, picker, prefix) {
        const centerX = list.width / 2
        const centerY = list.height / 2
        const initialIndex = list.indexAt(centerX, centerY + list.contentY)
        compare(list.contentY, 0)

        mouseWheel(list, centerX, centerY, 0, -120)
        wait(40)
        verify(list.contentY > 0, "滚轮必须滚动列表")

        const index = centerIndex(list)
        verify(index >= 0)
        verify(index !== initialIndex, "点击的应是滚轮后新出现的目录")

        mouseClick(list, centerX, centerY)
        wait(80)

        compare(picker.path.length, 2)
        compare(picker.targetId, prefix + index)
    }

    function test_task_picker_accepts_first_click_after_wheel() {
        const list = openTaskPicker()
        wheelThenClickFirstVisible(list, taskPicker, "task-folder-")
    }

    function test_single_wheel_notch_then_first_click() {
        const list = openTaskPicker()
        const x = list.width / 2
        const y = list.height / 2
        const initialIndex = list.indexAt(x, y + list.contentY)
        compare(list.contentY, 0)

        mouseWheel(list, x, y, 0, -120)
        wait(40)
        verify(list.contentY > 0)
        const index = centerIndex(list)
        verify(index >= 0)
        verify(index !== initialIndex)

        mouseClick(list, x, y)
        wait(80)
        compare(taskPicker.path.length, 2)
        compare(taskPicker.targetId, "task-folder-" + index)
    }

    function test_move_picker_accepts_first_click_after_wheel() {
        const list = openMovePicker()
        wheelThenClickFirstVisible(list, movePicker, "move-folder-")
    }

    function bottomThenClickFirstVisible(list, picker, prefix) {
        list.contentY = Math.max(0, list.contentHeight - list.height)
        wait(40)
        verify(list.contentY > 0)

        const x = list.width / 2
        const y = list.height / 2
        const index = list.indexAt(x, y + list.contentY)
        verify(index >= 0)

        mouseClick(list, x, y)
        wait(80)

        compare(picker.path.length, 2)
        compare(picker.targetId, prefix + index)
        compare(list.contentY, 0, "新目录加载完成后应从顶部显示")
        const firstItem = list.itemAtIndex(0)
        verify(firstItem)
        compare(String(firstItem.folderId), prefix + index + "-child-0")
        compare(Math.round(firstItem.y), 0,
                "新目录第一行必须落在视口顶部（不能残留上一层的 originY）")
    }

    function test_task_picker_entering_from_bottom_opens_at_top() {
        const list = openTaskPicker()
        bottomThenClickFirstVisible(list, taskPicker, "task-folder-")
    }

    function test_move_picker_entering_from_bottom_opens_at_top() {
        const list = openMovePicker()
        bottomThenClickFirstVisible(list, movePicker, "move-folder-")
    }

    function test_task_picker_delayed_load_does_not_jump_top() {
        const list = openTaskPicker()
        taskBackend.delayedLoads = true

        list.contentY = Math.max(0, list.contentHeight - list.height)
        wait(40)
        verify(list.contentY > 0)
        const beforeY = list.contentY
        const x = list.width / 2
        const y = list.height / 2
        const index = list.indexAt(x, y + list.contentY)
        verify(index >= 0)
        const clickedItem = list.itemAtIndex(index)
        verify(clickedItem)
        const clickedId = String(clickedItem.folderId)

        mouseClick(list, x, y)
        wait(20)
        compare(taskPicker.path.length, 2)
        verify(taskBackend.moveTargetsLoading)
        compare(Math.round(list.contentY), Math.round(beforeY),
                "加载子目录期间不能先跳回顶部")
        const loadingIndex = list.indexAt(x, y + list.contentY)
        const loadingItem = list.itemAtIndex(loadingIndex)
        verify(loadingItem)
        compare(String(loadingItem.folderId), clickedId,
                "等新目录到达前必须继续显示原位置的内容")

        wait(80)
        verify(!taskBackend.moveTargetsLoading)
        compare(list.contentY, 0,
                "子目录到达后应从新目录顶部显示")
        const firstItem = list.itemAtIndex(0)
        verify(firstItem)
        compare(String(firstItem.folderId),
                "task-folder-" + index + "-child-0")
    }

    function test_task_picker_height_stays_fixed_across_states() {
        const list = openTaskPicker()
        const initialHeight = taskPicker.height
        const initialListHeight = list.height
        verify(initialHeight > 0)

        // 空目录：提示条换文案，窗口和列表都不能变高变矮
        taskBackend.moveTargets = []
        taskBackend.moveTargetsRevision += 1
        taskBackend.changed()
        wait(40)
        compare(taskPicker.height, initialHeight, "空目录提示不能改变窗口高度")
        compare(list.height, initialListHeight, "空目录提示不能改变列表高度")

        // 出错：提示条换成错误 + 重试
        taskBackend.moveTargetsError = "读取目录失败"
        taskBackend.moveTargetsRevision += 1
        taskBackend.changed()
        wait(40)
        compare(taskPicker.height, initialHeight, "错误提示不能改变窗口高度")
        compare(list.height, initialListHeight, "错误提示不能改变列表高度")
    }

    function test_task_picker_long_to_short_folder_starts_at_top() {
        const list = openTaskPicker()
        list.contentY = Math.max(0, list.contentHeight - list.height)
        wait(40)
        verify(list.contentY > 0, "测试必须从长目录底部出发")

        taskBackend.shortChildren = true
        const x = list.width / 2
        const y = list.height / 2
        const index = list.indexAt(x, y + list.contentY)
        verify(index >= 0)
        mouseClick(list, x, y)
        wait(120)
        compare(taskPicker.path.length, 2)
        verify(list.contentHeight < list.height, "短目录必须短于视口")
        compare(Math.round(list.contentY), 0,
                "从长目录底部进入短目录也必须从顶部显示")
        const firstItem = list.itemAtIndex(0)
        verify(firstItem)
        compare(String(firstItem.folderId),
                "task-folder-" + index + "-child-0")
        compare(Math.round(firstItem.y), 0,
                "短目录第一行必须落在视口顶部（不能残留上一层的 originY）")
    }

    function test_move_picker_long_to_short_folder_starts_at_top() {
        const list = openMovePicker()
        list.contentY = Math.max(0, list.contentHeight - list.height)
        wait(40)
        verify(list.contentY > 0, "测试必须从长目录底部出发")

        moveBackend.shortChildren = true
        const x = list.width / 2
        const y = list.height / 2
        const index = list.indexAt(x, y + list.contentY)
        verify(index >= 0)
        mouseClick(list, x, y)
        wait(120)
        compare(movePicker.path.length, 2)
        verify(list.contentHeight < list.height, "短目录必须短于视口")
        compare(Math.round(list.contentY), 0,
                "从长目录底部进入短目录也必须从顶部显示")
        const firstItem = list.itemAtIndex(0)
        verify(firstItem)
        compare(String(firstItem.folderId),
                "move-folder-" + index + "-child-0")
        compare(Math.round(firstItem.y), 0,
                "短目录第一行必须落在视口顶部（不能残留上一层的 originY）")
    }

    function test_task_picker_enter_starts_at_top_and_go_up_restores() {
        const list = openTaskPicker()
        list.contentY = 200
        wait(30)
        taskPicker.enterFolder({ id: "task-folder-3", name: "任务文件夹 3" })
        wait(60)
        compare(taskPicker.path.length, 2)
        compare(Math.round(list.contentY), 0, "进入子目录必须从顶部显示")

        list.contentY = 240
        wait(30)
        taskPicker.goUp()
        wait(60)
        compare(taskPicker.targetId, "")
        tryCompare(list, "contentY", 200)
    }

    function test_move_picker_enter_starts_at_top_and_go_up_restores() {
        const list = openMovePicker()
        list.contentY = 200
        wait(30)
        movePicker.enterFolder({ id: "move-folder-3", name: "移动文件夹 3" })
        wait(60)
        compare(movePicker.path.length, 2)
        compare(Math.round(list.contentY), 0, "进入子目录必须从顶部显示")

        list.contentY = 240
        wait(30)
        movePicker.goUp()
        wait(60)
        compare(movePicker.targetId, "")
        tryCompare(list, "contentY", 200)
    }

    function test_picker_keeps_drag_scrolling() {
        const list = openTaskPicker()
        const before = list.contentY
        mouseDrag(list, list.width / 2, 200, 0, -120, Qt.LeftButton,
                  Qt.NoModifier, 20)
        wait(120)
        verify(list.contentY > before, "保留 Flickable 原生拖动")
    }

    function test_visible_row_click_still_opens() {
        const list = openTaskPicker()
        const x = list.width / 2
        const y = list.height / 2
        const index = list.indexAt(x, y + list.contentY)
        verify(index >= 0)
        mouseClick(list, x, y)
        wait(80)
        compare(taskPicker.path.length, 2)
        compare(taskPicker.targetId, "task-folder-" + index)
    }

    function test_wheel_keeps_scrolling_while_moving() {
        const list = openTaskPicker()
        const x = list.width / 2
        const y = list.height / 2
        mouseWheel(list, x, y, 0, -120)
        wait(40)
        const firstY = list.contentY
        verify(firstY > 0)

        mouseWheel(list, x, y, 0, -120)
        wait(80)
        verify(list.contentY > firstY, "惯性期间继续滚轮仍要滚动")
    }

    function test_wheel_does_not_cancel_native_inertia() {
        const list = openTaskPicker()
        const x = list.width / 2
        const y = list.height / 2

        mouseWheel(list, x, y, 0, -120)
        wait(40)
        const firstY = list.contentY
        verify(firstY > 0, "滚轮必须滚动列表")
        wait(100)
        verify(list.contentY > firstY, "滚轮后必须继续原生惯性滚动")
    }
}
