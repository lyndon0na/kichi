import QtQuick
import QtTest
import ".."

// 多选逻辑：Ctrl 切换、Shift 区间、全选、右键收窄、刷新后清理。
//
// 纯逻辑组件单独测，不依赖 FilesPage（那会把整个页面和桥接都拖进来）。
TestCase {
    name: "FileSelection"
    when: windowShown
    width: 400
    height: 300

    Component {
        id: selectionComponent
        FileSelection {}
    }

    property var selection: null

    readonly property var entries: [
        { id: "a", name: "a" },
        { id: "b", name: "b" },
        { id: "c", name: "c" },
        { id: "d", name: "d" }
    ]

    function init() {
        selection = selectionComponent.createObject(parent)
        verify(selection !== null, "FileSelection 创建失败")
    }

    function cleanup() {
        if (selection)
            selection.destroy()
        selection = null
    }

    function picked() {
        // QML 的 compare 不做数组深比较，统一转成字符串
        return selection.idList().sort().join(",")
    }

    function test_plain_click_selects_only_that_row() {
        selection.click("b", 1, Qt.NoModifier, entries)
        compare(picked(), "b")
        compare(selection.count, 1)
        verify(selection.isSelected("b"))
        verify(!selection.isSelected("a"))
        compare(selection.anchorId, "b")
    }

    function test_plain_click_collapses_a_multi_selection() {
        selection.click("a", 0, Qt.NoModifier, entries)
        selection.click("c", 2, Qt.ControlModifier, entries)
        compare(picked(), "a,c")
        selection.click("b", 1, Qt.NoModifier, entries)
        compare(picked(), "b", "普通点击把多选收窄成一条")
    }

    function test_ctrl_click_toggles() {
        selection.click("a", 0, Qt.NoModifier, entries)
        selection.click("c", 2, Qt.ControlModifier, entries)
        compare(picked(), "a,c")
        selection.click("a", 0, Qt.ControlModifier, entries)
        compare(picked(), "c", "再点一次取消选中")
    }

    function test_shift_click_selects_the_range() {
        selection.click("b", 1, Qt.NoModifier, entries)
        selection.click("d", 3, Qt.ShiftModifier, entries)
        compare(picked(), "b,c,d")
        compare(selection.anchorId, "b", "Shift 不移动锚点")
    }

    function test_ctrl_shift_adds_a_range_to_the_selection() {
        selection.click("a", 0, Qt.NoModifier, entries)
        selection.click("c", 2, Qt.ShiftModifier, entries)
        compare(picked(), "a,b,c")
        selection.click("d", 3, Qt.ControlModifier | Qt.ShiftModifier, entries)
        compare(picked(), "a,b,c,d", "Ctrl+Shift 在已有选择上并集")
    }

    function test_select_all_and_clear() {
        selection.selectAll(entries)
        compare(picked(), "a,b,c,d")
        compare(selection.count, 4)
        selection.clear()
        compare(selection.count, 0)
        compare(selection.singleId(), "")
    }

    function test_right_click_on_unselected_row_narrows_selection() {
        selection.click("a", 0, Qt.NoModifier, entries)
        selection.click("c", 2, Qt.ControlModifier, entries)
        selection.ensureSelected({ id: "b", name: "b" })
        compare(picked(), "b", "右键未选中的行：收窄成它")
        selection.ensureSelected({ id: "d", name: "d" })
        compare(picked(), "d")
    }

    function test_right_click_on_selected_row_keeps_the_selection() {
        selection.click("a", 0, Qt.NoModifier, entries)
        selection.click("c", 2, Qt.ControlModifier, entries)
        selection.ensureSelected({ id: "c", name: "c" })
        compare(picked(), "a,c", "右键已选中的行：保留整个选择")
    }

    function test_prune_drops_ids_that_are_gone() {
        selection.selectAll(entries)
        selection.prune([{ id: "a" }, { id: "d" }])
        compare(picked(), "a,d", "刷新后只留下仍存在的条目")
    }

    function test_prune_except_keeps_locked_rows_during_optimistic_removal() {
        selection.selectAll(entries)
        selection.pruneExcept([{ id: "a" }, { id: "c" }], ["b"])
        compare(picked(), "a,b,c", "正在处理的乐观移除行不能提前丢掉选择")
    }

    function test_remove_ids_only_drops_this_action_targets() {
        selection.selectAll(entries)
        selection.removeIds(["b", "d"])
        compare(picked(), "a,c", "并发动作只清理自己那批完成的目标")
    }

    function test_single_id_is_only_for_one_selection() {
        selection.click("a", 0, Qt.NoModifier, entries)
        compare(selection.singleId(), "a")
        selection.click("b", 1, Qt.ControlModifier, entries)
        compare(selection.singleId(), "", "多选时单条操作用不了")
    }

    function test_selection_is_remembered_per_directory() {
        selection.selectAll(entries)
        selection.rememberForKey("dir-a")
        selection.clear()

        selection.restoreForKey("dir-b", entries)
        compare(picked(), "", "没有记录的目录必须从空选择开始")
        compare(selection.anchorId, "")

        selection.restoreForKey("dir-a", entries)
        compare(picked(), "a,b,c,d")
        compare(selection.anchorId, "a", "锚点一并恢复")
    }

    function test_restore_drops_ids_that_left_the_directory() {
        selection.click("a", 0, Qt.NoModifier, entries)
        selection.click("c", 2, Qt.ControlModifier, entries)
        selection.rememberForKey("dir-a")
        selection.clear()

        selection.restoreForKey("dir-a", [{ id: "b" }, { id: "c" }])
        compare(picked(), "c", "已不存在的条目不能重新标成选中")
        compare(selection.anchorId, "c",
                "保存的锚点不存在时退回第一条选中项")
    }

    function test_selection_memory_evicts_the_oldest_directory() {
        selection.click("a", 0, Qt.NoModifier, entries)
        for (let i = 0; i <= selection.maxSavedDirs; ++i)
            selection.rememberForKey("dir-" + i)
        selection.clear()

        selection.restoreForKey("dir-0", entries)
        compare(picked(), "", "超过上限先淘汰最久未使用的目录")
        selection.restoreForKey("dir-" + selection.maxSavedDirs, entries)
        compare(picked(), "a", "最新目录仍在记忆里")
    }
}
