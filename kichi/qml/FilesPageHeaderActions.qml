import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 文件页标题行右侧的目录操作。添加菜单收纳目录内的创建动作，
// 任务页的离线标签仍负责任务查看与管理。刷新固定在最后。
RowLayout {
    id: root

    property var backend: null
    property var target: null
    property var newTaskTarget: null

    readonly property bool searching: backend !== null && backend.searchActive
    readonly property bool loading: backend !== null && backend.filesLoading
    readonly property bool manualRefreshing: backend !== null
                                            && backend.filesRefreshing === true
    readonly property bool canCreateFolder: backend !== null && target !== null
                                             && !searching && !loading
    readonly property bool canUpload: backend !== null && target !== null
                                      && !searching && !loading
    readonly property bool canCreateTask: backend !== null && newTaskTarget !== null

    readonly property KTheme theme: KTheme { parent: root }

    spacing: theme.gapSm

    KButton {
        id: addButton

        objectName: "filesAddButton"
        Layout.alignment: Qt.AlignVCenter
        text: qsTr("添加")
        iconName: "plus"
        iconSize: 14
        trailingIconName: "chevron-down"
        trailingIconSize: 11
        variant: "primary"
        implicitHeight: 34
        leftPadding: 8
        rightPadding: 8
        enabled: root.canUpload || root.canCreateFolder || root.canCreateTask
        showTextToolTip: false
        ToolTip.visible: hovered
        ToolTip.text: qsTr("上传文件 / 文件夹、新建文件夹或离线任务")
        Accessible.name: qsTr("添加")
        onClicked: root.openAddMenu()

        Menu {
            id: addMenu

            objectName: "filesAddMenu"
            parent: Overlay.overlay
            width: 220

            MenuItem {
                objectName: "filesAddUploadItem"
                text: qsTr("上传文件…")
                enabled: root.canUpload
                onTriggered: {
                    if (root.backend) {
                        root.backend.chooseUploadFiles(
                            root.backend.currentDirId, root.currentFolderPath())
                    }
                }

                ToolTip.visible: hovered
                ToolTip.text: qsTr("上传到当前文件夹，可一次选择多个文件")
            }

            MenuItem {
                objectName: "filesAddFolderUploadItem"
                text: qsTr("上传文件夹…")
                enabled: root.canUpload
                onTriggered: {
                    if (root.backend) {
                        root.backend.chooseUploadFolders(
                            root.backend.currentDirId, root.currentFolderPath())
                    }
                }

                ToolTip.visible: hovered
                ToolTip.text: qsTr("选择单个文件夹；多个文件夹可直接拖入窗口")
            }

            MenuItem {
                objectName: "filesAddNewFolderItem"
                text: qsTr("新建文件夹…")
                enabled: root.canCreateFolder
                onTriggered: {
                    if (root.target)
                        root.target.beginNewFolder()
                }
            }

            MenuItem {
                objectName: "filesAddNewTaskItem"
                text: qsTr("新建离线任务…")
                enabled: root.canCreateTask
                onTriggered: {
                    if (root.newTaskTarget)
                        root.newTaskTarget.openNewTaskDialog(
                            root.backend.currentDirId, root.currentFolderPath(),
                            root.backend.currentPath())
                }

                ToolTip.visible: hovered
                ToolTip.text: qsTr("任务产物会保存到当前文件夹")
            }
        }
    }

    ContentViewToggle {
        Layout.alignment: Qt.AlignVCenter
        backend: root.backend
        onViewRequested: function (view) {
            if (root.target)
                root.target.setContentView(view)
        }
    }

    KButton {
        objectName: "filesRefreshButton"
        Layout.alignment: Qt.AlignVCenter
        text: qsTr("刷新")
        iconName: "refresh"
        iconSize: 14
        busy: root.manualRefreshing
        variant: "ghost"
        implicitHeight: 34
        leftPadding: 8
        rightPadding: 8
        enabled: root.backend !== null && !root.searching && !root.loading
                 && !root.manualRefreshing
        showTextToolTip: false
        ToolTip.visible: hovered
        ToolTip.text: qsTr("刷新当前文件夹")
        onClicked: {
            if (root.target)
                root.target.refreshFiles()
        }
    }

    function openAddMenu() {
        if (!addButton.enabled)
            return
        const bottomRight = addButton.mapToItem(null, addButton.width,
                                                addButton.height)
        addMenu.popup(Qt.point(bottomRight.x - addMenu.width,
                               bottomRight.y + theme.gapXs))
    }

    function currentFolderPath() {
        const crumbs = root.backend && root.backend.breadcrumb
                     ? root.backend.breadcrumb : []
        const parts = []
        for (let i = 0; i < crumbs.length; i++)
            parts.push(String(crumbs[i]))
        return parts.length > 0 ? parts.join(" / ") : qsTr("全部文件")
    }
}
