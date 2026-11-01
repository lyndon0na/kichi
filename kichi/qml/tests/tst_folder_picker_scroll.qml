import QtQuick
import QtTest
import ".."

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
                for (let i = 0; i < 30; i++) {
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
        property var current: [{ id: "", name: "全部文件" }]

        signal changed()
        signal fileActionDone(string action, bool ok, string message)

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
                for (let i = 0; i < 30; i++) {
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
        taskBackend.pendingItems = []
        taskLoadTimer.stop()

        moveBackend.moveTargets = []
        moveBackend.moveTargetsRevision = 0
        moveBackend.moveTargetsLoading = false
        moveBackend.moveTargetsError = ""
        moveBackend.fileActionRunning = false
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

    function wheelThenClickFirstVisible(list, picker, prefix) {
        const centerX = list.width / 2
        const centerY = list.height / 2
        const initialIndex = list.indexAt(centerX, centerY + list.contentY)
        compare(list.contentY, 0)

        mouseWheel(list, centerX, centerY, 0, -120)
        wait(40)
        verify(list.contentY > 0, "滚轮必须滚动列表")

        const index = list.indexAt(centerX, centerY + list.contentY)
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
        const index = list.indexAt(x, y + list.contentY)
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
