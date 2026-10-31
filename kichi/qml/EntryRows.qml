import QtQuick

// 文件类列表 / 网格共用的稳定模型。
//
// Rust 每次刷新都会发布一份新的条目数组。直接把数组交给 ListView / GridView
// 会整表重灌并把 contentY 甩回顶部；这里按 id 做差异，只增删 / 移动 / 替换
// 真正变化的行。条目本身放在 entry 角色里，避免把 id 当成 ListModel 字段。
Item {
    id: root

    visible: false
    width: 0
    height: 0

    property var source: []
    property alias model: rows
    property alias count: rows.count

    ListModel {
        id: rows
    }

    onSourceChanged: sync()
    Component.onCompleted: sync()

    /// 按 source 的顺序和内容同步内部 ListModel。
    function sync() {
        const wanted = root.source || []
        const wantedIds = ({})
        for (let i = 0; i < wanted.length; ++i) {
            const entry = wanted[i]
            const key = entry && entry.id !== undefined ? String(entry.id) : ""
            if (key.length > 0)
                wantedIds[key] = true
        }

        // 1. 先删掉已经不存在的行，再从旧表末尾补齐新行。
        for (let stale = rows.count - 1; stale >= 0; --stale) {
            if (wantedIds[String(rows.get(stale).key)] === undefined)
                rows.remove(stale)
        }
        const present = ({})
        for (let p = 0; p < rows.count; ++p)
            present[String(rows.get(p).key)] = true
        for (let add = 0; add < wanted.length; ++add) {
            const entry = wanted[add]
            const key = entry && entry.id !== undefined ? String(entry.id) : ""
            if (key.length === 0 || present[key] !== undefined)
                continue
            rows.append({
                key: key,
                entry: entry,
                dateGroup: entry && entry.dateGroup !== undefined
                           ? String(entry.dateGroup) : ""
            })
            present[key] = true
        }

        // 2. 从尾往前对齐顺序。后面的位置先固定，目标行一定在它前面；
        // 这样一次结构变化只需要一次 move，滚动位置不会被反复搅动。
        for (let target = wanted.length - 1; target >= 0 && target < rows.count; --target) {
            const entry = wanted[target]
            const key = entry && entry.id !== undefined ? String(entry.id) : ""
            if (key.length === 0 || String(rows.get(target).key) === key)
                continue
            let from = -1
            for (let scan = target - 1; scan >= 0; --scan) {
                if (String(rows.get(scan).key) === key) {
                    from = scan
                    break
                }
            }
            if (from >= 0)
                rows.move(from, target, 1)
        }

        // 3. 位置一致后再替换内容；只改字段不会动滚动位置。
        for (let at = 0; at < wanted.length && at < rows.count; ++at) {
            const entry = wanted[at]
            if (!entry || !sameEntry(rows.get(at).entry, entry))
                rows.set(at, {
                    key: entry && entry.id !== undefined
                         ? String(entry.id) : "",
                    entry: entry,
                    dateGroup: entry && entry.dateGroup !== undefined
                               ? String(entry.dateGroup) : ""
                })
        }

        // 防御性清理，正常情况下上面已经对齐。
        while (rows.count > wanted.length)
            rows.remove(rows.count - 1)
    }

    function sameEntry(a, b) {
        if (a === b)
            return true
        if (!a || !b)
            return false
        for (const key in a) {
            if (!(key in b) || a[key] !== b[key])
                return false
        }
        for (const key in b) {
            if (!(key in a))
                return false
        }
        return true
    }
}
