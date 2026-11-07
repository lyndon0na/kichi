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

    /// 目录 id → { ids, anchorId }；只在本次运行内保留。
    property var savedByDir: ({})
    /// 目录的使用顺序，超出上限时淘汰最久未使用的。
    property var savedOrder: []
    readonly property int maxSavedDirs: 20

    /// 离开目录前调用：把当前选择记到这个目录名下。
    /// 选择为空时直接删掉记录，避免空目录长期占位。
    function rememberForKey(key) {
        const dirKey = String(key)
        if (count === 0) {
            if (savedByDir[dirKey] === undefined)
                return
            const next = copyDirMap(savedByDir)
            delete next[dirKey]
            savedByDir = next
            const order = savedOrder.slice()
            const at = order.indexOf(dirKey)
            if (at >= 0)
                order.splice(at, 1)
            savedOrder = order
            return
        }
        const next = copyDirMap(savedByDir)
        next[dirKey] = {
            ids: copy(selectedIds),
            anchorId: anchorId
        }
        savedByDir = next
        touchKey(dirKey)
    }

    /// 回到目录后调用：只恢复目标目录里仍存在的 id；
    /// 没有记录就清空当前选择，不能把上一个目录的选择带进来。
    function restoreForKey(key, entries) {
        const dirKey = String(key)
        const saved = savedByDir[dirKey]
        if (!saved || !saved.ids) {
            clear()
            return
        }
        const present = ({})
        for (let i = 0; i < (entries ? entries.length : 0); ++i) {
            const id = entries[i] ? entries[i].id : ""
            if (id)
                present[id] = true
        }
        const next = ({})
        let first = ""
        const ids = Object.keys(saved.ids)
        for (let i = 0; i < ids.length; ++i) {
            if (present[ids[i]]) {
                next[ids[i]] = true
                if (first.length === 0)
                    first = ids[i]
            }
        }
        selectedIds = next
        const anchor = saved.anchorId
        anchorId = anchor && next[anchor] ? anchor : first
        touchKey(dirKey)
    }

    function clearMemory() {
        savedByDir = ({})
        savedOrder = []
    }

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

    /// 只选中一个条目（离线任务定位、搜索结果定位这类一次性定位用）。
    function selectOnly(id) {
        const next = ({})
        if (id)
            next[id] = true
        selectedIds = next
        anchorId = id || ""
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
        pruneExcept(entries, [])
    }

    /// 列表刷新后丢掉已经不存在的 id，但保留仍被写操作占用的行：
    /// 乐观移除 / 回滚期间它们暂时不在列表里，失败后还要恢复选择。
    function pruneExcept(entries, keepIds) {
        const present = ({})
        for (let i = 0; i < (entries ? entries.length : 0); i++) {
            const id = entries[i] ? entries[i].id : ""
            if (id)
                present[id] = true
        }
        for (let k = 0; k < (keepIds ? keepIds.length : 0); k++)
            present[String(keepIds[k])] = true
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

    function removeIds(ids) {
        if (!ids || ids.length === 0)
            return
        const next = copy(selectedIds)
        let changed = false
        for (let i = 0; i < ids.length; i++) {
            if (next[ids[i]]) {
                delete next[ids[i]]
                changed = true
            }
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

    function copyDirMap(map) {
        const next = ({})
        const keys = Object.keys(map)
        for (let i = 0; i < keys.length; i++)
            next[keys[i]] = map[keys[i]]
        return next
    }

    /// 记录一次使用，并按上限淘汰最久未使用的目录。
    function touchKey(key) {
        const order = savedOrder.slice()
        const at = order.indexOf(key)
        if (at >= 0)
            order.splice(at, 1)
        order.push(key)
        const evicted = []
        while (order.length > maxSavedDirs)
            evicted.push(order.shift())
        if (evicted.length > 0) {
            const next = copyDirMap(savedByDir)
            for (let i = 0; i < evicted.length; i++)
                delete next[evicted[i]]
            savedByDir = next
        }
        savedOrder = order
    }
}
