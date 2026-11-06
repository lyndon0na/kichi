import QtQuick
import QtTest
import ".."

TestCase {
    name: "MagnetFilesDialog"
    when: windowShown
    width: 900
    height: 700

    QtObject {
        id: backend

        property var magnetFiles: []
        property string magnetRootName: "示例磁链"
        property string magnetSummary: ""
        property string magnetFilesError: ""
        property bool magnetFilesLoading: false

        property var lastIndices: []
        property string lastFolderId: ""
        property var lastKeepStructure: undefined

        function currentPath() {
            return [{ id: "", name: "全部文件" }]
        }

        function saveMagnetFiles(indices, folderId, keepStructure) {
            lastIndices = indices.slice()
            lastFolderId = folderId
            lastKeepStructure = keepStructure
        }
    }

    MagnetFilesDialog {
        id: dialog
        backend: backend
    }

    SignalSpy {
        id: savedSpy
        target: dialog
        signalName: "saved"
    }

    property var list: null
    property var tree: null
    property var countLabel: null
    property var destinationLabel: null
    property var emptyLabel: null
    property var footerOk: null
    property var footerCancel: null

    readonly property var sampleRows: [
        { index: 0, name: "第 1 集.mkv", path: "Season 1", folders: ["Season 1"],
          size: "1.2 GB", note: "", available: true },
        { index: 2, name: "第 2 集.mkv", path: "Season 1", folders: ["Season 1"],
          size: "1.1 GB", note: "", available: true },
        { index: 5, name: "花絮.mp4", path: "Extras", folders: ["Extras"],
          size: "80 MB", note: "云端暂无缓存，无法秒传", available: false }
    ]

    function init() {
        backend.magnetFiles = sampleRows
        backend.magnetRootName = "示例磁链"
        backend.magnetSummary = "共 3 个文件 · 2.4 GB；2 个可秒传，其余暂不可秒传"
        backend.magnetFilesError = ""
        backend.magnetFilesLoading = false
        backend.lastIndices = []
        backend.lastFolderId = ""
        backend.lastKeepStructure = undefined
        savedSpy.clear()
        if (dialog.visible)
            dialog.close()
    }

    function cleanup() {
        if (dialog.visible)
            dialog.close()
    }

    function open() {
        dialog.openFor("", "全部文件", [])
        tryCompare(dialog, "visible", true)
        tree = findChild(dialog, "magnetFilesTree")
        list = findChild(dialog, "magnetFilesList")
        countLabel = findChild(dialog, "magnetSelectionCount")
        destinationLabel = findChild(dialog, "magnetDestinationPath")
        emptyLabel = findChild(dialog, "magnetFilesEmpty")
        footerOk = findChild(dialog, "dialogFooterOk")
        footerCancel = findChild(dialog, "dialogFooterCancel")
    }

    function rowFor(key) {
        const index = tree.rowByKey[key]
        if (index === undefined || index < 0)
            return null
        return list.model.get(index)
    }

    function test_only_cached_files_are_selected_by_default() {
        open()
        // Season 1 文件夹 + 2 个文件、Extras 文件夹 + 1 个文件
        compare(list.count, 5)
        compare(tree.rowCount, 5)
        compare(dialog.availableCount, 2)
        compare(dialog.checkedCount, 2)
        compare(dialog.allChecked, true)
        compare(countLabel.text, "已选 2 / 3")
        compare(footerOk.enabled, true)

        compare(rowFor("d:Season 1").nodeState, Qt.Checked)
        compare(rowFor("f:0").selected, true)
        compare(rowFor("f:2").selected, true)
        compare(rowFor("d:Extras").nodeState, Qt.Unchecked)
        compare(rowFor("f:5").selected, false)
    }

    function test_select_all_never_checks_unavailable_rows() {
        open()
        tree.setAllChecked(false)
        compare(dialog.checkedCount, 0)
        compare(dialog.allChecked, false)
        compare(footerOk.enabled, false)
        compare(rowFor("d:Season 1").nodeState, Qt.Unchecked)
        compare(rowFor("f:0").selected, false)
        compare(rowFor("f:2").selected, false)

        tree.setAllChecked(true)
        compare(dialog.checkedCount, 2)
        compare(rowFor("d:Season 1").nodeState, Qt.Checked)
        compare(rowFor("f:0").selected, true)
        compare(rowFor("f:2").selected, true)
        compare(rowFor("f:5").selected, false,
                "没有 GCID 的文件不能被勾选")
        compare(rowFor("f:5").available, false)
        compare(rowFor("d:Extras").nodeState, Qt.Unchecked)
    }

    function test_delegate_receives_checked_state() {
        open()
        const index = tree.rowByKey["f:0"]
        tryVerify(function () { return list.itemAtIndex(index) !== null })
        const delegate = list.itemAtIndex(index)
        compare(delegate.nodeState, Qt.Checked)
        compare(delegate.selected, true)
    }

    function test_unchecking_one_file_marks_folder_partial() {
        open()
        tree.toggleRow(tree.rowByKey["f:0"])

        compare(rowFor("f:0").selected, false)
        compare(rowFor("f:2").selected, true)
        compare(rowFor("d:Season 1").nodeState, Qt.PartiallyChecked)
        compare(dialog.checkedCount, 1)
        compare(dialog.allChecked, false)
        compare(countLabel.text, "已选 1 / 3")
    }

    function test_folder_toggle_selects_and_clears_descendants() {
        open()
        const folder = tree.rowByKey["d:Season 1"]

        tree.toggleRow(folder)
        compare(rowFor("f:0").selected, false)
        compare(rowFor("f:2").selected, false)
        compare(rowFor("d:Season 1").nodeState, Qt.Unchecked)
        compare(dialog.checkedCount, 0)

        tree.toggleRow(folder)
        compare(rowFor("f:0").selected, true)
        compare(rowFor("f:2").selected, true)
        compare(rowFor("d:Season 1").nodeState, Qt.Checked)
        compare(dialog.checkedCount, 2)
    }

    function test_collapsing_hides_descendants_without_losing_selection() {
        open()
        const folder = tree.rowByKey["d:Season 1"]

        tree.toggleExpanded(folder)
        compare(rowFor("d:Season 1").expanded, false)
        compare(rowFor("f:0").shown, false)
        compare(rowFor("f:2").shown, false)
        compare(rowFor("d:Extras").shown, true, "同级文件夹不受影响")
        compare(dialog.checkedCount, 2, "折叠不改变已选文件")

        tree.setAllChecked(false)
        compare(dialog.checkedCount, 0)

        tree.toggleExpanded(folder)
        compare(rowFor("f:0").shown, true)
        compare(rowFor("f:2").shown, true)
    }

    function test_array_like_rows_from_backend_are_accepted() {
        // cxx-qt 的 QVariantList 进 QML 后是「带 length 的数组型对象」，
        // Array.isArray 会判成 false；这里用同形状的普通对象锁住兼容性。
        const folders = { length: 1, 0: "S" }
        backend.magnetFiles = {
            length: 2,
            0: { index: 0, name: "a.mkv", path: "S", folders: folders,
                 size: "1 MB", note: "", available: true },
            1: { index: 1, name: "b.mkv", path: "S", folders: folders,
                 size: "1 MB", note: "", available: true }
        }
        open()

        compare(tree.totalFiles, 2)
        compare(tree.rowCount, 3, "文件夹 + 2 个文件")
        compare(dialog.checkedCount, 2)
        compare(rowFor("d:S").nodeState, Qt.Checked)
        compare(rowFor("f:1").selected, true)
    }

    function test_unavailable_folder_is_not_selectable() {
        open()
        tree.toggleRow(tree.rowByKey["d:Extras"])
        compare(rowFor("f:5").selected, false)
        compare(rowFor("d:Extras").nodeState, Qt.Unchecked)
        compare(dialog.checkedCount, 2, "点不可秒传的文件夹不应改变选择")
    }

    function test_save_sends_listing_indices_and_target_folder() {
        open()
        dialog.targetFolderId = "folder-1"
        dialog.targetFolderPath = "全部文件 / 动画"
        compare(destinationLabel.text, "保存到：全部文件 / 动画")

        dialog.save()
        compare(backend.lastIndices.length, 2)
        compare(backend.lastIndices[0], 0)
        compare(backend.lastIndices[1], 2)
        compare(backend.lastFolderId, "folder-1")
        compare(backend.lastKeepStructure, true, "默认保留目录结构")
        compare(dialog.visible, false, "提交后立即收起，进度交给底栏")
        compare(savedSpy.count, 1, "收起时通知新任务弹窗一起关闭")
    }

    function test_keep_structure_can_be_turned_off() {
        open()
        const check = findChild(dialog, "magnetKeepStructure")
        verify(check !== null)
        compare(check.checked, true)
        compare(dialog.keepStructure, true)

        check.checked = false
        compare(check.checked, false, "复选框应已取消")
        compare(dialog.keepStructure, false)

        dialog.save()
        compare(backend.lastKeepStructure, false, "关闭后要求平铺保存")
        compare(backend.lastIndices.length, 2)
    }

    function test_cancel_closes_without_saving() {
        open()
        mouseClick(footerCancel)
        tryCompare(dialog, "visible", false)
        compare(backend.lastIndices.length, 0)
        compare(savedSpy.count, 0, "取消不关新任务弹窗")
    }

    function test_no_selection_does_nothing() {
        open()
        tree.setAllChecked(false)
        compare(footerOk.enabled, false)
        dialog.save()
        compare(dialog.visible, true)
        compare(backend.lastIndices.length, 0)
        compare(savedSpy.count, 0)
    }

    function test_empty_listing_cannot_be_saved() {
        backend.magnetFiles = []
        open()
        compare(list.count, 0)
        compare(emptyLabel.visible, true)
        compare(dialog.checkedCount, 0)
        compare(footerOk.enabled, false)
    }
}
