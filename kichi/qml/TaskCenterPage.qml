import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 任务页：本机传输与离线任务共用一个入口，但保留两张独立表格。
// 两个子页各自维护数据、轮询、滚动和操作；这里只负责切换 active 状态。
Item {
    id: page

    property var backend: null
    property bool active: false
    property int viewportWidth: 0
    property string currentTab: "transfers"

    readonly property TransfersPage transfersView: transfersPage
    readonly property TasksPage tasksView: tasksPage
    readonly property int transferActiveCount: transfersPage.activeCount
    readonly property int offlineActiveCount: tasksPage.activeCount
    /// 当前标签页是否正内联显示播放加载态（传输标签没有内联播放）。
    readonly property bool showsInlinePlayLoading: page.tasksView.showsInlinePlayLoading

    signal openFolderRequested(string fileId)
    signal revealLocationRequested(string fileId)
    signal playRequested(string fileId, point position)

    UiMetrics { id: metrics }
    KTheme { id: theme }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        TaskCenterTabs {
            objectName: "taskCenterTabs"
            Layout.fillWidth: true
            Layout.leftMargin: metrics.pageMargin
            Layout.rightMargin: metrics.pageMargin + metrics.scrollBarSlot
            Layout.topMargin: metrics.pageMargin
            Layout.preferredHeight: 32
            currentTab: page.currentTab
            transferCount: page.transferActiveCount
            offlineCount: page.offlineActiveCount
            onTabRequested: function (tab) {
                page.showTab(tab)
            }
        }

        StackLayout {
            id: tabStack

            objectName: "taskCenterContentStack"
            Layout.fillWidth: true
            Layout.fillHeight: true
            currentIndex: page.currentTab === "offline" ? 1 : 0

            TransfersPage {
                id: transfersPage

                backend: page.backend
                active: page.active && page.currentTab === "transfers"
                viewportWidth: page.viewportWidth
            }

            TasksPage {
                id: tasksPage

                backend: page.backend
                active: page.active && page.currentTab === "offline"
                onOpenFolderRequested: function (fileId) {
                    page.openFolderRequested(fileId)
                }
                onRevealLocationRequested: function (fileId) {
                    page.revealLocationRequested(fileId)
                }
                onPlayRequested: function (fileId, position) {
                    page.playRequested(fileId, position)
                }
            }
        }
    }

    onCurrentTabChanged: {
        if (page.active)
            tabContentFade.restart()
    }

    NumberAnimation {
        id: tabContentFade

        target: tabStack
        property: "opacity"
        from: 0
        to: 1
        duration: theme.motionNormal
        easing.type: Easing.OutCubic
    }

    function showTab(tab) {
        if (tab === "offline" || tab === "transfers")
            page.currentTab = tab
    }

    function showTransfers() {
        page.currentTab = "transfers"
    }

    function showOffline() {
        page.currentTab = "offline"
    }

    function showTaskEntry() {
        page.showTab(page.transferActiveCount > 0 || page.offlineActiveCount === 0
                     ? "transfers" : "offline")
    }

    function focusTransfer(sourceId) {
        page.showTransfers()
        page.transfersView.focusSource(sourceId)
    }

    /// 文件页「新建离线任务…」也走这里；打开弹窗不强制切换当前页。
    function openNewTaskDialog(targetFolderId, targetFolderPath, startPath) {
        page.tasksView.openNewTaskDialog(targetFolderId, targetFolderPath, startPath)
    }
}
