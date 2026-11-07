import QtQuick

// 文件页的交互逻辑：目录导航、显式定位、键盘快捷键、文件操作完成后的收尾。
//
// 从 FilesPage.qml 拆出来（页面已超行数软上限）。这里不画任何东西，
// 只持有状态并调用 backend；页面负责把 fileList / menu / selection 接进来。
// 滚动 / 选中按目录记忆和导航恢复在 FileNavigationState.qml。
QtObject {
    id: ctl

    objectName: "filePageInteraction"

    property var backend: null
    property var selection: null
    property var fileList: null
    property var menu: null
    property var flashTimer: null
    /// 当前视图真正显示的条目：普通目录或搜索结果。
    property var entries: []
    property bool searching: false
    /// 搜索结果已发出目录跳转，但目标目录仍在加载：暂时不接受重复导航。
    property bool navigationLocked: false
    /// 当前活动视图："list" / "grid"；滚动位置按「目录 + 视图」分别记住
    property string viewMode: "list"

    /// 目录导航状态：滚动 / 选中记忆、进入子目录归顶、返回恢复。
    property FileNavigationState navigation: FileNavigationState {
        backend: ctl.backend
        fileList: ctl.fileList
        selection: ctl.selection
        entries: ctl.entries
        searching: ctl.searching
        navigationLocked: ctl.navigationLocked
        viewMode: ctl.viewMode
        revealScrollActive: ctl.pendingRevealId.length > 0
                             || ctl.pendingCreatedRevealId.length > 0
                             || ctl.createdFlashId.length > 0
        revealSelectionActive: ctl.pendingRevealId.length > 0
                               || ctl.pendingCreatedRevealId.length > 0
    }

    signal folderOpenRequested(string id)

    /// 离线任务 / 搜索结果定位：等目录模型真正显示后再选中目标。
    property string pendingRevealId: ""
    /// 新建文件夹的临时行出现后，等模型回填再滚动到它。
    property string pendingCreatedRevealId: ""
    /// 当前正在强调的新建文件夹 id（不改选择）。
    property string createdFlashId: ""

    function entryForId(id) {
        for (let i = 0; i < ctl.entries.length; i++) {
            if (ctl.entries[i] && ctl.entries[i].id === id)
                return ctl.entries[i]
        }
        return null
    }

    /// 选择里哪些是文件夹（选择器据此禁止「移进自己」）
    function folderIdsFor(ids) {
        const folders = []
        if (!ctl.backend || !ids)
            return folders
        const wanted = ({})
        for (let i = 0; i < ids.length; i++)
            wanted[ids[i]] = true
        const entries = ctl.entries
        for (let i = 0; i < entries.length; i++) {
            const entry = entries[i]
            if (entry && wanted[entry.id] && entry.kind === "folder")
                folders.push(entry.id)
        }
        return folders
    }

    // —— 目录导航（页面入口，细节在 FileNavigationState）——

    function rememberScroll() {
        navigation.rememberScroll()
    }

    function rememberSelection() {
        navigation.rememberSelection()
    }

    function restoreScrollNow() {
        navigation.restoreScrollNow()
    }

    function prepareViewSwitch(view) {
        navigation.prepareViewSwitch(view)
    }

    function noteFilesRevision() {
        navigation.noteFilesRevision()
    }

    function clearSelectionMemory() {
        navigation.clearSelectionMemory()
    }

    function beginSearch() {
        navigation.beginSearch()
    }

    function endSearch() {
        navigation.endSearch()
    }

    function navigateToDir(id) {
        if (!ctl.backend || ctl.backend.filesLoading === true)
            return
        ctl.clearCreatedFlash()
        navigation.navigateToDir(id)
    }

    /// 搜索结果进文件夹：同样从顶部开始，并让 endSearch 跳过原目录恢复。
    function beginSearchChildNavigation() {
        if (!ctl.backend)
            return
        ctl.clearCreatedFlash()
        navigation.beginSearchChildNavigation()
    }

    function goParent() {
        if (!ctl.backend)
            return
        ctl.clearCreatedFlash()
        navigation.goParent()
    }

    function goCrumb(index) {
        if (!ctl.backend)
            return
        ctl.clearCreatedFlash()
        navigation.goCrumb(index)
    }

    function refreshFiles() {
        if (!ctl.backend)
            return
        navigation.refreshFiles()
    }

    /// 目录模型加载完成后，按 id 选中并滚动到目标条目。
    function revealEntryById(id) {
        const target = String(id || "")
        if (target.length === 0)
            return
        pendingRevealId = target
        navigation.cancelScrollRestore()
        navigation.cancelSelectionRestore()
        Qt.callLater(ctl.tryRevealPending)
    }

    function tryRevealPending() {
        if (pendingRevealId.length === 0 || !ctl.backend || !ctl.fileList
                || ctl.backend.filesLoading === true || ctl.navigationLocked)
            return false
        const entries = ctl.entries
        let index = -1
        for (let i = 0; i < (entries ? entries.length : 0); i++) {
            if (entries[i] && String(entries[i].id) === pendingRevealId) {
                index = i
                break
            }
        }
        if (index < 0 || ctl.fileList.contentHeight <= 0)
            return false

        if (ctl.selection)
            ctl.selection.selectOnly(pendingRevealId)
        ctl.fileList.currentIndex = index
        ctl.fileList.positionViewAtIndex(
            index, ctl.viewMode === "grid" ? GridView.Contain : ListView.Contain)
        pendingRevealId = ""
        return true
    }

    /// 新建文件夹的临时行一出现就定位并强调，不进入目录，也不改变当前选择。
    function flashCreatedEntry(id) {
        const target = String(id || "")
        if (target.length === 0 || ctl.searching)
            return
        pendingCreatedRevealId = target
        createdFlashId = ""
        if (ctl.flashTimer)
            ctl.flashTimer.stop()
        Qt.callLater(ctl.tryRevealCreated)
    }

    function tryRevealCreated() {
        if (pendingCreatedRevealId.length === 0 || !ctl.backend || !ctl.fileList
                || ctl.backend.filesLoading === true || ctl.navigationLocked)
            return false
        const entries = ctl.entries
        let index = -1
        for (let i = 0; i < (entries ? entries.length : 0); i++) {
            if (entries[i] && String(entries[i].id) === pendingCreatedRevealId) {
                index = i
                break
            }
        }
        if (index < 0 || ctl.fileList.contentHeight <= 0)
            return false

        const target = pendingCreatedRevealId
        pendingCreatedRevealId = ""
        createdFlashId = target
        navigation.cancelScrollRestore()
        ctl.fileList.currentIndex = index
        ctl.fileList.positionViewAtIndex(
            index, ctl.viewMode === "grid" ? GridView.Contain : ListView.Contain)
        if (ctl.flashTimer)
            ctl.flashTimer.restart()
        return true
    }

    /// 真实 id 回来后原位接管强调；首次定位尚未完成时才补一次滚动。
    function resolveCreatedEntry(id) {
        const target = String(id || "")
        if (target.length === 0 || ctl.searching)
            return
        if (pendingCreatedRevealId.length > 0) {
            pendingCreatedRevealId = target
            Qt.callLater(ctl.tryRevealCreated)
            return
        }
        if (createdFlashId.length === 0)
            return
        const entries = ctl.entries
        for (let i = 0; i < (entries ? entries.length : 0); i++) {
            if (entries[i] && String(entries[i].id) === target) {
                if (ctl.fileList)
                    ctl.fileList.currentIndex = i
                break
            }
        }
        createdFlashId = target
        if (ctl.flashTimer)
            ctl.flashTimer.restart()
    }

    function clearCreatedFlash() {
        pendingCreatedRevealId = ""
        createdFlashId = ""
        if (ctl.flashTimer)
            ctl.flashTimer.stop()
    }

    onViewModeChanged: {
        if (pendingRevealId.length > 0)
            Qt.callLater(ctl.tryRevealPending)
        else if (pendingCreatedRevealId.length > 0)
            Qt.callLater(ctl.tryRevealCreated)
    }

    onEntriesChanged: {
        if (pendingRevealId.length > 0)
            Qt.callLater(ctl.tryRevealPending)
        if (pendingCreatedRevealId.length > 0)
            Qt.callLater(ctl.tryRevealCreated)
    }

    /// 文件操作结果回到界面：乐观模型已原位更新，移走 / 删掉的清空选择。
    function handleFileActionDone(action, ok, ids) {
        if (action === "folder" && !ok
                && (!ids || ids.length === 0 || ids.indexOf(ctl.createdFlashId) >= 0))
            ctl.clearCreatedFlash()
        if (!ok)
            return
        if (action === "trash" || action === "delete" || action === "move") {
            if (ids && ids.length > 0)
                ctl.selection.removeIds(ids)
            else
                ctl.selection.prune(ctl.entries)
        }
    }

    // —— 键盘快捷键（挂在列表控件上，避开 Wayland 下窗口级 Keys 不可靠）——

    function handleKey(event) {
        if (!ctl.backend)
            return
        if (ctl.navigationLocked) {
            event.accepted = true
            return
        }
        const ctrl = (event.modifiers & Qt.ControlModifier) !== 0
        if (ctrl && event.key === Qt.Key_A) {
            ctl.selection.selectAll(ctl.entries)
            event.accepted = true
            return
        }
        if (event.key === Qt.Key_Delete) {
            const ids = ctl.selection.idList()
            if (ids.length === 0)
                return
            const entry = ctl.entryForId(ids[0])
            if (entry) {
                ctl.menu.openDeleteFor(entry, ids)
                event.accepted = true
            }
            return
        }
        if (event.key === Qt.Key_F2) {
            const id = ctl.selection.singleId()
            const entry = id ? ctl.entryForId(id) : null
            if (entry) {
                ctl.menu.openRenameFor(entry, [id])
                event.accepted = true
            }
            return
        }
        if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
            const id = ctl.selection.singleId()
            const entry = id ? ctl.entryForId(id) : null
            // 只对文件夹生效：回车不应误触发播放 / 下载
            if (entry && entry.kind === "folder") {
                if (ctl.searching)
                    ctl.folderOpenRequested(entry.id)
                else
                    ctl.navigateToDir(entry.id)
                event.accepted = true
            }
            return
        }
        if (event.key === Qt.Key_Backspace) {
            if (ctl.searching)
                return
            const count = ctl.backend.breadcrumb.length
            if (count > 1) {
                ctl.goParent()
                event.accepted = true
            }
        }
    }
}
