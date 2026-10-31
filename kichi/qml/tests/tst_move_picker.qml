import QtQuick
import QtTest
import ".."

// 移动 / 复制的目标文件夹选择器。
//
// 锁住几条容易回归的规则：源目录不能当目标、源文件夹不能进入、
// 请求被接受后立即关窗、列表只按 revision 重灌。
TestCase {
    name: "MovePickerDialog"
    when: windowShown
    width: 900
    height: 700

    QtObject {
        id: backend

        property var moveTargets: []
        property int moveTargetsRevision: 0
        property bool moveTargetsLoading: false
        property string moveTargetsError: ""
        property bool fileActionRunning: false
        property var current: [
            { id: "", name: "全部文件" },
            { id: "A", name: "A" }
        ]

        property int moveCount: 0
        property int copyCount: 0
        property var lastIds: []
        property string lastTargetId: ""

        signal changed()
        signal fileActionDone(string action, bool ok, string message)

        function currentPath() {
            return current
        }

        function loadMoveTargets(parentId) {
            if (parentId === "A")
                moveTargets = [{ id: "B", name: "B" }, { id: "F", name: "F" }]
            else if (parentId === "B")
                moveTargets = []
            else
                moveTargets = [{ id: "A", name: "A" }]
            moveTargetsRevision += 1
            moveTargetsLoading = false
            moveTargetsError = ""
            changed()
        }

        function cancelMoveTargets() {
        }

        function moveEntries(ids, targetId, targetName) {
            moveCount += 1
            lastIds = ids.slice()
            lastTargetId = targetId
            fileActionRunning = true
            changed()
            return true
        }

        function copyEntries(ids, targetId, targetName) {
            copyCount += 1
            lastIds = ids.slice()
            lastTargetId = targetId
            fileActionRunning = true
            changed()
            return true
        }

        function finish(action, ok, message) {
            fileActionRunning = false
            fileActionDone(action, ok, message)
            changed()
        }
    }

    Component {
        id: pickerComponent
        MovePickerDialog {}
    }

    property var picker: null

    function init() {
        backend.moveTargets = []
        backend.moveTargetsRevision = 0
        backend.moveTargetsLoading = false
        backend.moveTargetsError = ""
        backend.fileActionRunning = false
        backend.current = [{ id: "", name: "全部文件" }, { id: "A", name: "A" }]
        backend.moveCount = 0
        backend.copyCount = 0
        backend.lastIds = []
        backend.lastTargetId = ""
        picker = pickerComponent.createObject(parent, {
            visible: false, backend: backend
        })
        verify(picker !== null, "MovePickerDialog 创建失败")
    }

    function cleanup() {
        if (picker)
            picker.destroy()
        picker = null
    }

    function openMove(entry, ids, folders) {
        picker.openForMove(entry, ids, folders)
        wait(60)
    }

    function okButton() {
        return findChild(picker, "dialogFooterOk")
    }

    function test_source_directory_cannot_be_the_target() {
        backend.current = [{ id: "", name: "全部文件" }]
        openMove({ id: "f1", name: "file.mkv", kind: "file" }, ["f1"], [])

        verify(picker.opened, "选择器打开了")
        compare(picker.targetId, "", "从根目录打开时起点是根")
        compare(picker.sourceParentId, "", "源目录是根")
        compare(okButton().enabled, false, "目标就是源目录：确定置灰")
    }

    function test_moving_out_to_parent_is_allowed() {
        openMove({ id: "f1", name: "file.mkv", kind: "file" }, ["f1"], [])
        compare(picker.targetId, "", "起点是当前目录的上一级（根）")
        compare(picker.sourceParentId, "A")
        compare(okButton().enabled, true, "移到上一级是合法目标")
    }

    function test_source_directory_can_be_traversed_but_not_chosen() {
        openMove({ id: "f1", name: "file.mkv", kind: "file" }, ["f1"], [])
        picker.enterFolder({ id: "A", name: "A" })
        wait(40)
        compare(picker.targetId, "A")
        compare(okButton().enabled, false, "经过源目录时不能选它")

        picker.enterFolder({ id: "B", name: "B" })
        wait(40)
        compare(picker.targetId, "B")
        compare(okButton().enabled, true, "进入子文件夹后可以选")
    }

    function test_source_folder_is_blocked_and_cannot_be_entered() {
        openMove({ id: "F", name: "源文件夹", kind: "folder" }, ["F"], ["F"])
        picker.enterFolder({ id: "A", name: "A" })
        wait(40)

        verify(picker.blocked("F"), "源文件夹要置灰")
        const before = picker.path.length
        picker.enterFolder({ id: "F", name: "源文件夹" })
        wait(20)
        compare(picker.path.length, before, "源文件夹不能进入（否则能绕到它的子目录）")
    }

    function test_move_calls_backend_and_closes_before_network_result() {
        openMove({ id: "f1", name: "file.mkv", kind: "file" }, ["f1"], [])
        picker.enterFolder({ id: "A", name: "A" })
        wait(40)
        picker.enterFolder({ id: "B", name: "B" })
        wait(40)
        compare(okButton().enabled, true)

        mouseClick(okButton())
        wait(40)
        compare(backend.moveCount, 1)
        compare(backend.lastIds.join(","), "f1")
        compare(backend.lastTargetId, "B")
        verify(!picker.opened, "本地源行已移除，选择器立即关闭")

        backend.finish("move", false, "目标里已有同名条目")
        wait(40)
        verify(!picker.opened, "网络失败由底栏提示，不能重新弹窗")
    }

    function test_success_closes_the_picker() {
        openMove({ id: "f1", name: "file.mkv", kind: "file" }, ["f1"], [])
        picker.enterFolder({ id: "A", name: "A" })
        wait(40)
        picker.enterFolder({ id: "B", name: "B" })
        wait(40)
        mouseClick(okButton())
        wait(40)
        verify(!picker.opened, "请求被接受后立即关闭")
        backend.finish("move", true, "已移动")
        wait(40)
        verify(!picker.opened, "成功后保持关闭")
    }

    function test_copy_mode_uses_copy_endpoint() {
        picker.openForCopy({ id: "f1", name: "file.mkv", kind: "file" }, ["f1"], [])
        wait(60)
        compare(picker.title, "复制到…")
        picker.enterFolder({ id: "A", name: "A" })
        wait(40)
        picker.enterFolder({ id: "B", name: "B" })
        wait(40)
        mouseClick(okButton())
        wait(40)
        compare(backend.copyCount, 1)
        compare(backend.moveCount, 0)
        compare(backend.lastTargetId, "B")
    }

    function test_list_error_is_shown() {
        openMove({ id: "f1", name: "file.mkv", kind: "file" }, ["f1"], [])
        backend.moveTargetsError = "读取目录失败"
        backend.moveTargetsRevision += 1
        backend.changed()
        wait(40)
        compare(findChild(picker, "movePickerListError").text, "读取目录失败")
        verify(findChild(picker, "movePickerRetry") !== null, "有重试入口")
        compare(findChild(picker, "movePickerEmpty").visible, false,
                "有错误时不要同时显示空状态")
    }

    function test_batch_subject_shows_count() {
        openMove({ id: "f1", name: "a.mkv", kind: "file" },
                 ["f1", "f2", "f3"], [])
        picker.enterFolder({ id: "A", name: "A" })
        wait(40)
        picker.enterFolder({ id: "B", name: "B" })
        wait(40)
        mouseClick(okButton())
        wait(40)
        compare(backend.lastIds.length, 3, "批量 id 一次提交")
    }
}
