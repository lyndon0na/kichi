import QtQuick
import QtQuick.Controls

// 文件页面包屑行右端的「资料库」入口（2026-11-05，方案 C）：
// 最近保存 / 播放历史 / 已加星标从侧栏收进这里，不新增行高；
// 文件页和库页都常驻，进库后能直接切到另一个库。
// 停在库页时按钮保持强调色；选中态只跟 currentPage，不参与点击勾选。
KButton {
    id: root

    property int currentPage: 0
    signal pageRequested(int pageIndex)
    readonly property alias menu: libraryMenu

    PageIndex { id: pages }
    KTheme { id: theme }

    objectName: "libraryMenuButton"
    text: qsTr("资料库")
    iconName: "library"
    iconSize: 14
    trailingIconName: "chevron-down"
    trailingIconSize: 11
    variant: "ghost"
    pageActive: pages.libraryPages.indexOf(root.currentPage) >= 0
    implicitHeight: 28
    leftPadding: 8
    rightPadding: 8
    font.pixelSize: theme.fontMeta
    showTextToolTip: false
    ToolTip.visible: hovered
    ToolTip.text: qsTr("打开资料库：最近保存 / 播放历史 / 已加星标")
    Accessible.name: qsTr("资料库")
    onClicked: root.openMenu()

    Menu {
        id: libraryMenu

        objectName: "libraryMenu"
        // 挂在窗口 Overlay 上，popup 才能直接吃场景坐标（与页头菜单一致）
        parent: Overlay.overlay
        width: 180

        // MenuItem 没有互斥组，点击会改写 checked 并断掉绑定；
        // 每次弹出时按当前页重读（同 FileSortMenu）。
        onAboutToShow: {
            recentItem.checked = root.currentPage === pages.recentSaved
            historyItem.checked = root.currentPage === pages.playbackHistory
            starredItem.checked = root.currentPage === pages.starred
        }

        MenuItem {
            id: recentItem
            objectName: "libraryMenuRecentSaved"
            text: qsTr("最近保存")
            checkable: true
            onTriggered: root.pageRequested(pages.recentSaved)
        }

        MenuItem {
            id: historyItem
            objectName: "libraryMenuPlaybackHistory"
            text: qsTr("播放历史")
            checkable: true
            onTriggered: root.pageRequested(pages.playbackHistory)
        }

        MenuItem {
            id: starredItem
            objectName: "libraryMenuStarred"
            text: qsTr("已加星标")
            checkable: true
            onTriggered: root.pageRequested(pages.starred)
        }
    }

    function openMenu() {
        const bottomRight = root.mapToItem(null, root.width, root.height)
        libraryMenu.popup(Qt.point(bottomRight.x - libraryMenu.width,
                                   bottomRight.y + theme.gapXs))
    }
}
