import QtQuick

// 移动 / 复制选择器与离线任务保存位置选择器共用的目录导航状态。
//
// 只负责：当前路径、按目录记住的滚动位置、以及把 backend 的
// moveTargets / moveTargetsRevision 灌进列表模型。
// 弹窗自己的标题、提示条、delegate、确定 / 取消回调仍留在各自文件里。
// 只有 MovePicker 需要 canEnter（源文件夹不可进入）；传 null 表示全部可进。
QtObject {
    id: nav

    property var backend: null
    property var fileList: null
    property var path: []
    /// 返回 false 的目录不可进入
    property var canEnter: null

    property int seenRevision: -1
    /// 下一次列表就绪后要恢复的位置（< 0 回到顶部）；
    /// -2 表示未指定——同目录后台更新沿用当前列表位置，不跳顶部
    property real restoreY: -2

    property ListModel foldersModel: ListModel {}

    property KScrollMemory scroll: KScrollMemory {
        flickable: nav.fileList
    }

    property Connections backendConnections: Connections {
        target: nav.backend

        function onChanged() {
            nav.syncFolders()
        }
    }

    readonly property string targetId: path.length > 0
                                      ? String(path[path.length - 1].id || "") : ""

    /// 打开选择器时清空滚动记忆，下一次加载从顶部开始。
    function reset() {
        seenRevision = -1
        scroll.clearKeys()
        restoreY = -1
    }

    /// 记住当前目录列表的滚动位置（导航前调用）
    function rememberListScroll() {
        if (fileList)
            scroll.rememberKey(targetId)
    }

    function reload() {
        if (fileList)
            fileList.beginDirectoryLoad()
        if (backend)
            backend.loadMoveTargets(targetId, false)
    }

    /// 手动刷新：跳过本地快照强制联网，刷新后留在原位置
    function refresh() {
        if (!backend || backend.moveTargetsLoading)
            return
        if (fileList)
            fileList.beginDirectoryLoad()
        backend.loadMoveTargets(targetId, true)
    }

    function goUp() {
        if (path.length <= 1)
            return
        rememberListScroll()
        path = path.slice(0, path.length - 1)
        restoreY = scroll.valueFor(targetId, -1)
        reload()
    }

    function enterFolder(folder) {
        if (!folder || (canEnter && !canEnter(folder.id)))
            return
        rememberListScroll()
        restoreY = -1
        path = path.concat([{
            id: String(folder.id || ""),
            name: String(folder.name || folder.id || "")
        }])
        reload()
    }

    function syncFolders() {
        if (!backend || backend.moveTargetsRevision === seenRevision)
            return
        seenRevision = backend.moveTargetsRevision
        const pending = restoreY
        restoreY = -2
        // 同目录的后台补全不能把用户刚滚到的位置冲回顶部；
        // 位置必须在回顶之前取，否则拿到的已经是 0
        const restore = pending === -2
                      ? (fileList ? fileList.contentY : 0) : pending
        // 列表缩短时 Qt 会残留旧 originY，替换模型前先回顶
        if (fileList)
            fileList.prepareDirectoryReplace()
        foldersModel.clear()
        const list = backend.moveTargets || []
        for (let i = 0; i < list.length; i++) {
            foldersModel.append({
                folderId: String(list[i].id || ""),
                folderName: String(list[i].name || "")
            })
        }
        if (fileList)
            fileList.finishDirectoryLoad(restore)
    }
}
