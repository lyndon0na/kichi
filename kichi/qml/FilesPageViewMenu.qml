import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 文件页的视图列表：列表 / 网格。
//
// 按钮放在 main.qml 的页头；真正切换由 FilesPage 接住，这样切换前能先记住
// 当前视图的滚动位置，再恢复目标视图的位置。搜索视图固定用结果列表，
// 所以搜索进行中这里禁用。
Item {
    id: root

    property var backend: null
    readonly property string currentView: root.backend
                                            && root.backend.fileView === "grid"
                                          ? "grid" : "list"

    signal viewRequested(string view)

    implicitWidth: 84
    implicitHeight: 30

    KTheme { id: theme }

    KButton {
        id: button

        objectName: "viewMenuButton"

        anchors.fill: parent
        variant: "ghost"
        iconName: root.currentView === "grid" ? "layout-grid" : "list"
        iconSize: 15
        text: root.currentView === "grid" ? qsTr("网格") : qsTr("列表")
        implicitHeight: 30
        leftPadding: 8
        rightPadding: 8
        font.pixelSize: 12
        enabled: root.backend !== null && root.backend.searchActive !== true
        Accessible.name: qsTr("文件显示方式")
        ToolTip.visible: hovered
        ToolTip.text: !enabled
                      ? qsTr("搜索结果固定用列表显示")
                      : qsTr("切换列表或网格视图")
        onClicked: root.openMenu()
    }

    Menu {
        id: menu

        objectName: "viewMenu"

        parent: Overlay.overlay
        width: 168
        padding: 4

        MenuItem {
            objectName: "viewListOption"
            text: qsTr("列表")
            Accessible.name: qsTr("列表视图")
            contentItem: RowLayout {
                spacing: 8

                NavIcon {
                    Layout.preferredWidth: 14
                    Layout.preferredHeight: 14
                    name: "check"
                    color: root.currentView === "list" ? theme.accent
                                                       : "transparent"
                }

                Label {
                    Layout.fillWidth: true
                    text: qsTr("列表")
                    font.pixelSize: 13
                    color: theme.textPrimary
                }
            }
            onTriggered: root.viewRequested("list")
        }

        MenuItem {
            objectName: "viewGridOption"
            text: qsTr("网格")
            Accessible.name: qsTr("网格视图")
            contentItem: RowLayout {
                spacing: 8

                NavIcon {
                    Layout.preferredWidth: 14
                    Layout.preferredHeight: 14
                    name: "check"
                    color: root.currentView === "grid" ? theme.accent
                                                       : "transparent"
                }

                Label {
                    Layout.fillWidth: true
                    text: qsTr("网格")
                    font.pixelSize: 13
                    color: theme.textPrimary
                }
            }
            onTriggered: root.viewRequested("grid")
        }
    }

    function openMenu() {
        if (!button.enabled)
            return
        const point = button.mapToItem(null, button.width, button.height)
        menu.popup(Qt.point(Math.max(0, point.x - menu.width), point.y + 4))
    }
}
