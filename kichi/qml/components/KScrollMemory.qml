import QtQuick

// 列表滚动位置记忆。
//
// 单值模式：页面切走时记住 contentY，切回来经 `begin()` 恢复；列表行分批
// 灌入时 contentHeight 会先给出中间值，最多重试 4 次。
// 多键模式：目录 / 视图各记一份，`rememberKey` 复制 map 后写键（触发属性变更），
// `clampTo` 把值夹进视图边界。pending 标志、reveal 优先级仍由调用方掌握。
QtObject {
    id: memory

    /// 要读写的滚动视图（ListView、GridView 或 FolderPickerListView 等）。
    property var flickable: null

    property real savedY: 0
    property bool pending: false
    property int attempts: 0

    /// key → contentY；复制后整体替换，避免原地改 map 不触发变更。
    property var savedByKey: ({})

    readonly property int maxAttempts: 4

    function remember() {
        if (flickable)
            savedY = flickable.contentY
    }

    function cancel() {
        pending = false
        attempts = 0
    }

    /// 开始一次带重试的恢复；调用时机与原先的 callLater 一致。
    function begin() {
        pending = true
        attempts = 0
        Qt.callLater(memory.restore)
    }

    function restore() {
        if (!pending || !flickable || flickable.contentHeight <= 0)
            return
        var target = clampTo(flickable, savedY)
        flickable.contentY = target
        if (target >= savedY || attempts >= maxAttempts) {
            pending = false
            attempts = 0
            return
        }
        attempts += 1
        Qt.callLater(memory.restore)
    }

    function rememberKey(key) {
        if (!flickable)
            return
        var next = ({})
        var keys = Object.keys(savedByKey)
        for (var i = 0; i < keys.length; i++)
            next[keys[i]] = savedByKey[keys[i]]
        next[key] = flickable.contentY
        savedByKey = next
    }

    function valueFor(key, fallback) {
        var value = savedByKey[key]
        return value === undefined ? fallback : value
    }

    function clearKeys() {
        savedByKey = ({})
    }

    /// 夹到视图边界内；内容比视口短时回 0。
    function clampTo(view, y) {
        if (!view)
            return Math.max(0, y)
        var maxY = Math.max(0, view.contentHeight - view.height)
        return Math.max(0, Math.min(y, maxY))
    }

    function applyTo(view, y) {
        if (view)
            view.contentY = clampTo(view, y)
    }
}
