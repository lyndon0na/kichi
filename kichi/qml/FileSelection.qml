pragma ComponentBehavior: Bound

import QtQuick

// 文件列表的多选状态（纯逻辑，不画界面）。
//
// 从 FilesPage.qml 拆出来：选中集合、Ctrl/Shift 区间、右键收窄都在一起，
// 页面本体只负责把点击和按键转进来。`selectedIds` 是对象 map（id → true），
// 每次变更都整体重新赋值，QML 绑定才会刷新。
QtObject {
    id: selection

    property var selectedIds: ({})
    /// Shift 区间的起点（id）。排序 / 刷新后仍按 id 找位置
    property string anchorId: ""

    readonly property int count: Object.keys(selectedIds).length

    function isSelected(id) {
        return !!id && selectedIds[id] === true
    }

    function idList() {
        return Object.keys(selectedIds)
    }

    /// 恰好选中一条时返回它的 id，否则空串（重命名 / 播放这类单条操作据此置灰）
    function singleId() {
        const ids = Object.keys(selectedIds)
        return ids.length === 1 ? ids[0] : ""
    }

    function clear() {
        selectedIds = ({})
        anchorId = ""
    }

    function selectAll(entries) {
        const next = ({})
        for (let i = 0; i < (entries ? entries.length : 0); i++) {
            const id = entries[i] ? entries[i].id : ""
            if (id)
                next[id] = true
        }
        selectedIds = next
        anchorId = entries && entries.length > 0 ? (entries[0].id || "") : ""
    }

    /// 一次左键点击。`modifiers` 来自 MouseArea（Ctrl / Shift 位）。
    function click(id, index, modifiers, entries) {
        if (!id || index < 0)
            return
        const ctrl = (modifiers & Qt.ControlModifier) !== 0
        const shift = (modifiers & Qt.ShiftModifier) !== 0

        if (shift) {
            const anchor = indexOf(anchorId, entries)
            const from = Math.min(anchor >= 0 ? anchor : index, index)
            const to = Math.max(anchor >= 0 ? anchor : index, index)
            // Ctrl+Shift 在已有选择上追加区间，普通 Shift 只保留区间
            const next = ctrl ? copy(selectedIds) : ({})
            for (let i = from; i <= to; i++) {
                const current = entries && entries[i] ? entries[i].id : ""
                if (current)
                    next[current] = true
            }
            selectedIds = next
            if (anchor < 0)
                anchorId = id
            return
        }

        if (ctrl) {
            const next = copy(selectedIds)
            if (next[id])
                delete next[id]
            else
                next[id] = true
            selectedIds = next
            anchorId = id
            return
        }

        const next = ({})
        next[id] = true
        selectedIds = next
        anchorId = id
    }

    /// 右键点到未选中的行：像文件管理器一样收窄成这一条；
    /// 点在已选中的行上则保留整个选择。
    function ensureSelected(entry) {
        if (!entry || !entry.id)
            return
        if (selectedIds[entry.id] === true)
            return
        const next = ({})
        next[entry.id] = true
        selectedIds = next
        anchorId = entry.id
    }

    /// 列表刷新后丢掉已经不存在的 id（例如在别处被删除）。
    function prune(entries) {
        const present = ({})
        for (let i = 0; i < (entries ? entries.length : 0); i++) {
            const id = entries[i] ? entries[i].id : ""
            if (id)
                present[id] = true
        }
        const next = ({})
        let changed = false
        const ids = Object.keys(selectedIds)
        for (let i = 0; i < ids.length; i++) {
            if (present[ids[i]])
                next[ids[i]] = true
            else
                changed = true
        }
        if (changed)
            selectedIds = next
    }

    function indexOf(id, entries) {
        if (!id)
            return -1
        for (let i = 0; i < (entries ? entries.length : 0); i++) {
            if (entries[i] && entries[i].id === id)
                return i
        }
        return -1
    }

    function copy(map) {
        const next = ({})
        const keys = Object.keys(map)
        for (let i = 0; i < keys.length; i++)
            next[keys[i]] = true
        return next
    }
}
