import QtQuick

// 当前被写操作占用的条目 id。调用点只禁用自己这批目标，
// 不再因为别的条目正在改名 / 移入回收站 / 清空回收站而整页变灰。
QtObject {
    id: locks

    property var backend: null
    readonly property var ids: backend && backend.fileActionIds
                                    ? backend.fileActionIds : []

    function has(id) {
        const key = String(id || "")
        if (key.length === 0)
            return false
        for (let i = 0; i < ids.length; ++i) {
            if (String(ids[i]) === key)
                return true
        }
        return false
    }

    function any(values) {
        if (!values)
            return false
        for (let i = 0; i < values.length; ++i) {
            if (has(values[i]))
                return true
        }
        return false
    }
}
