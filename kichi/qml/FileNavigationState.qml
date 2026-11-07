import QtQuick

// 文件页的目录导航状态（纯逻辑，不画界面）。
//
// 滚动位置和选中状态都按「目录 + 视图」记住：进入子目录一律从顶部显示，
// 返回上级 / 点面包屑 / 退出搜索时恢复该目录离开时的位置。显式定位
// （离线任务、搜索结果、新建文件夹强调）期间不恢复旧状态。
QtObject {
    id: nav

    property var backend: null
    property var fileList: null
    property var selection: null
    property var entries: []
    property bool searching: false
    property bool navigationLocked: false
    property string viewMode: "list"
    /// 显式定位期间为 true：旧的滚动 / 选择不能盖掉定位结果。
    property bool revealScrollActive: false
    property bool revealSelectionActive: false

    // —— 滚动位置记忆（目录 id + 视图 → contentY）——

    property KScrollMemory scrollMemory: KScrollMemory {
        flickable: nav.fileList
    }
    property bool restoreScrollPending: false
    property real pendingScrollY: -1
    property string pendingViewMode: ""
    /// 进入子目录：本次导航固定从顶部开始，不读目标目录的历史位置。
    property bool pendingScrollTop: false
    /// 恢复较深位置时列表可能还没完成新模型布局，按帧重试一小段时间。
    property int scrollRestoreAttempts: 0
    readonly property int maxScrollRestoreAttempts: 20

    property Timer scrollRetryTimer: Timer {
        interval: 16
        repeat: false
        onTriggered: nav.restoreScrollNow()
    }

    // —— 选中状态记忆（目录 id → 选中集合）——

    property bool restoreSelectionPending: false
    /// 这次搜索退出来自「点搜索结果里的文件夹」，endSearch 不能按原目录覆盖。
    property bool searchChildNavigation: false

    property int seenFilesRevision: -1

    function scrollKeyForDir(dir, view) {
        return String(dir) + ":" + String(view)
    }

    function scrollKeyFor(view) {
        const scope = nav.searching
                    ? "search"
                    : (nav.backend ? nav.backend.currentDirId : "")
        return scrollKeyForDir(scope, view)
    }

    function rememberScroll() {
        if (!nav.backend || !nav.fileList)
            return
        scrollMemory.rememberKey(scrollKeyFor(viewMode))
    }

    /// 搜索开始 / 结束时按当前目录保存，不能因为 searching 已经翻转而写进
    /// 临时的 "search" 键。
    function rememberScrollForDir() {
        if (!nav.backend || !nav.fileList)
            return
        scrollMemory.rememberKey(
                    scrollKeyForDir(nav.backend.currentDirId, viewMode))
    }

    function cancelScrollRestore() {
        restoreScrollPending = false
        pendingScrollTop = false
        pendingScrollY = -1
        pendingViewMode = ""
        scrollRestoreAttempts = 0
        scrollRetryTimer.stop()
    }

    function restoreScrollNow() {
        // 定位目标时以“让目标可见”为准，旧的目录滚动位置不能随后把它盖掉。
        if (revealScrollActive)
            return
        if (pendingScrollY < 0)
            return
        if (pendingViewMode.length > 0 && pendingViewMode !== viewMode)
            return
        // 搜索 / 搜索结果进目录的保留期 / 首次加载期间，旧的 entries 还在，
        // 此时恢复会先套到上一个目录上。
        if (!nav.backend || nav.searching || nav.navigationLocked
                || nav.backend.filesLoading === true)
            return
        // 空目录或尚未布局：清掉待恢复值，别留到下一次无关刷新再跳
        if (nav.entries.length === 0) {
            pendingScrollY = -1
            pendingViewMode = ""
            return
        }
        if (!nav.fileList || nav.fileList.contentHeight <= 0)
            return
        const target = pendingScrollY
        scrollMemory.applyTo(nav.fileList, target)
        // 新目录的内容高度可能还没算完（回到长目录时尤其明显），先别丢掉目标值。
        if (target > 0 && nav.fileList.contentY + 0.5 < target
                && scrollRestoreAttempts < maxScrollRestoreAttempts) {
            if (!scrollRetryTimer.running) {
                scrollRestoreAttempts += 1
                scrollRetryTimer.restart()
            }
            return
        }
        pendingScrollY = -1
        pendingViewMode = ""
        scrollRestoreAttempts = 0
        scrollRetryTimer.stop()
        if (target === 0) {
            // Qt 6.11 在列表高度变化后可能异步恢复旧 contentY（docs/pitfalls.md
            // 第 57 条），下一帧再压一次，确保进入子目录真的停在第一行。
            nav.fileList.positionViewAtBeginning()
            nav.fileList.contentY = 0
            nav.fileList.forceLayout()
            Qt.callLater(nav.enforceScrollTop)
        }
    }

    function enforceScrollTop() {
        if (!nav.fileList || pendingScrollY >= 0 || pendingViewMode.length > 0)
            return
        nav.fileList.positionViewAtBeginning()
        if (nav.fileList.contentY !== 0)
            nav.fileList.contentY = 0
    }

    /// 保存当前目录的选择；搜索中调用时跳过，避免把搜索结果记成目录选择。
    function rememberSelectionForDir() {
        if (!nav.backend || !nav.selection)
            return
        nav.selection.rememberForKey(String(nav.backend.currentDirId))
    }

    function rememberSelection() {
        if (nav.searching)
            return
        rememberSelectionForDir()
    }

    function cancelSelectionRestore() {
        restoreSelectionPending = false
    }

    function restoreSelectionNow() {
        if (!restoreSelectionPending || !nav.backend || !nav.selection)
            return
        if (nav.searching || nav.navigationLocked
                || nav.backend.filesLoading === true)
            return
        // 显式定位会自己选中目标，不能被目录记忆盖掉。
        if (revealSelectionActive)
            return
        nav.selection.restoreForKey(String(nav.backend.currentDirId),
                                    nav.entries)
        restoreSelectionPending = false
    }

    function clearSelectionMemory() {
        if (nav.selection)
            nav.selection.clearMemory()
    }

    /// 切换列表 / 网格：先把当前视图的位置记下来，等目标视图真正可见后再恢复。
    function prepareViewSwitch(view) {
        if (!nav.backend)
            return
        pendingScrollY = scrollMemory.valueFor(scrollKeyFor(view), 0)
        pendingViewMode = view
        restoreScrollPending = false
        scrollRestoreAttempts = 0
        scrollRetryTimer.stop()
        Qt.callLater(nav.restoreScrollNow)
    }

    onViewModeChanged: {
        if (!nav.revealScrollActive && nav.pendingViewMode === nav.viewMode)
            Qt.callLater(nav.restoreScrollNow)
    }

    onEntriesChanged: {
        // revision 没推进（例如进入内容相同的目录）时也要有机会恢复。
        Qt.callLater(nav.restoreScrollNow)
        Qt.callLater(nav.restoreSelectionNow)
    }

    onSearchingChanged: {
        if (!nav.searching) {
            Qt.callLater(nav.restoreScrollNow)
            Qt.callLater(nav.restoreSelectionNow)
        }
    }

    onNavigationLockedChanged: {
        if (!nav.navigationLocked) {
            Qt.callLater(nav.restoreScrollNow)
            Qt.callLater(nav.restoreSelectionNow)
        }
    }

    function noteFilesRevision() {
        if (!nav.backend)
            return
        // 搜索批次更新不一定改 filesRevision；结果模型一变就先清掉已消失的选择。
        if (nav.searching) {
            if (nav.selection)
                nav.selection.pruneExcept(nav.entries, nav.backend.fileActionIds)
            return
        }
        const revision = nav.backend.filesRevision
        if (revision === seenFilesRevision)
            return
        seenFilesRevision = revision
        if (nav.selection)
            nav.selection.pruneExcept(nav.backend.files, nav.backend.fileActionIds)
        if (restoreScrollPending) {
            restoreScrollPending = false
            pendingScrollY = pendingScrollTop
                           ? 0
                           : scrollMemory.valueFor(scrollKeyFor(viewMode), 0)
            pendingScrollTop = false
            pendingViewMode = ""
            scrollRestoreAttempts = 0
            scrollRetryTimer.stop()
            // 列表布局在模型更新之后才完成；contentHeight 变化时会再试一次
            Qt.callLater(nav.restoreScrollNow)
        }
        if (restoreSelectionPending)
            Qt.callLater(nav.restoreSelectionNow)
    }

    // —— 导航（先记滚动位置和选择；进入子目录固定回顶）——

    /// 进入子目录的公共入口（普通目录和搜索结果都走这里）：
    /// 记住父目录的滚动 / 选择；目标目录固定从顶部、空选择开始。
    /// 子目录上一次退出的选择不再恢复（选择只用于向上返回）。
    function beginChildNavigation() {
        if (!nav.backend)
            return
        searchChildNavigation = false
        if (!nav.searching) {
            rememberScroll()
            rememberSelection()
        }
        pendingScrollTop = true
        pendingScrollY = 0
        pendingViewMode = ""
        scrollRestoreAttempts = 0
        scrollRetryTimer.stop()
        restoreScrollPending = true
        restoreSelectionPending = false
        if (nav.selection)
            nav.selection.clear()
    }

    function navigateToDir(id) {
        if (!nav.backend || nav.backend.filesLoading === true)
            return
        beginChildNavigation()
        nav.backend.openDir(id)
    }

    /// 搜索结果进文件夹：同样按子目录归顶，但要让 endSearch 知道这次不能
    /// 恢复原目录的位置。结果过期 / 读失败时取消这次待恢复。
    function beginSearchChildNavigation() {
        if (!nav.backend)
            return
        const origin = String(nav.backend.currentDirId)
        beginChildNavigation()
        searchChildNavigation = true
        Qt.callLater(function () {
            if (!nav.searchChildNavigation || !nav.backend)
                return
            if (String(nav.backend.currentDirId) === origin
                    && nav.backend.filesLoading !== true) {
                nav.searchChildNavigation = false
                nav.cancelScrollRestore()
                nav.cancelSelectionRestore()
            }
        })
    }

    /// 返回上一级 / 点面包屑：恢复目标层级离开时的滚动位置和选择。
    function beginParentNavigation() {
        if (!nav.backend)
            return
        searchChildNavigation = false
        rememberScroll()
        rememberSelection()
        pendingScrollTop = false
        pendingScrollY = -1
        pendingViewMode = ""
        scrollRestoreAttempts = 0
        scrollRetryTimer.stop()
        restoreScrollPending = true
        restoreSelectionPending = true
    }

    function goParent() {
        if (!nav.backend)
            return
        beginParentNavigation()
        nav.backend.openParent()
    }

    function goCrumb(index) {
        if (!nav.backend)
            return
        beginParentNavigation()
        nav.backend.openCrumb(index)
    }

    function refreshFiles() {
        if (!nav.backend)
            return
        nav.backend.clearStatusMessage()
        rememberScroll()
        rememberSelection()
        pendingScrollTop = false
        pendingScrollY = -1
        pendingViewMode = ""
        restoreScrollPending = true
        nav.backend.refreshFiles()
    }

    // —— 搜索进出：保存 / 恢复当前目录的滚动和选择 ——

    function beginSearch() {
        if (!nav.backend)
            return
        searchChildNavigation = false
        rememberScrollForDir()
        rememberSelectionForDir()
        if (nav.selection)
            nav.selection.clear()
        cancelScrollRestore()
        cancelSelectionRestore()
    }

    function endSearch() {
        if (!nav.backend)
            return
        // 从搜索结果进文件夹时，beginChildNavigation 已经排好目标目录归顶 /
        // 选择恢复，这里不能再按原目录覆盖。
        if (searchChildNavigation) {
            searchChildNavigation = false
            return
        }
        pendingScrollTop = false
        pendingScrollY = scrollMemory.valueFor(
                    scrollKeyForDir(nav.backend.currentDirId, viewMode), 0)
        pendingViewMode = ""
        scrollRestoreAttempts = 0
        scrollRetryTimer.stop()
        restoreScrollPending = false
        restoreSelectionPending = true
        Qt.callLater(nav.restoreScrollNow)
        Qt.callLater(nav.restoreSelectionNow)
    }
}
