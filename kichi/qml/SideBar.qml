import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 登录后的固定导航栏：我的文件 / 传输任务 / 分享 / 回收站。
// 三个资料库（最近保存 / 播放历史 / 已加星标）是文件页的子页，入口在
// 文件页面包屑行（LibraryMenuButton.qml），不占侧栏；停在库页时仍选中「我的文件」。
// 设置收进账号行右侧；退出登录在设置页「账号」一节。
//
// 视觉：侧栏底色比内容区低半档，导航项是圆角卡片；选中态用「淡强调色底 +
// 3px 竖条 + 强调色图标」，不铺满饱和色块（与文件列表的选中语言一致）。
// 全侧栏只有账号区上方一条分隔线。
Rectangle {
    id: bar

    property var backend: null
    property int currentIndex: 0
    property Item selectedNavItem: null
    property bool hasNavSelection: false
    /// 设置对话框是否显示：账号行齿轮的点亮状态由窗口可见性决定
    property bool settingsActive: false
    /// 本机进行中的下载 / 上传批次数与离线未结束任务数；「传输任务」徽标用
    property int localTaskCount: 0
    property int offlineTaskCount: 0

    /// 主导航高亮用的索引：资料库三页属于文件页，选中态落回「我的文件」
    readonly property int selectionIndex: pages.libraryPages.indexOf(bar.currentIndex) >= 0
                                          ? pages.files : bar.currentIndex
    readonly property int taskCount: bar.localTaskCount + bar.offlineTaskCount
    readonly property string taskCountText: bar.taskCount > 99
                                            ? "99+" : String(bar.taskCount)

    signal pageRequested(int pageIndex)
    signal settingsRequested()

    KTheme { id: theme }
    PageIndex { id: pages }

    color: theme.surfaceSheet

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: theme.gapMd
        spacing: theme.gapXs

        Repeater {
            // 显示顺序与 main.qml 的页面索引解耦；page 始终对应实际页面。
            model: [
                { "page": pages.files, "label": qsTr("我的文件"), "icon": "folder" },
                { "page": pages.transfers, "label": qsTr("传输任务"), "icon": "transfer" },
                { "page": pages.shares, "label": qsTr("分享"), "icon": "share" },
                { "page": pages.trash, "label": qsTr("回收站"), "icon": "trash" }
            ]

            delegate: Button {
                id: navButton
                required property var modelData

                // Button 的 checkable 会在点击时自行改写 checked，并永久切断与当前页的
                // 绑定；选中态只认 currentIndex，避免连点当前入口后高亮消失。
                readonly property bool navSelected: bar.selectionIndex === modelData.page

                objectName: "sidebarNav_" + modelData.page
                Layout.fillWidth: true
                Layout.preferredHeight: 40
                padding: 0
                leftPadding: 12
                rightPadding: 12
                Component.onCompleted: bar.updateNavSelection(navButton, navSelected)
                onNavSelectedChanged: bar.updateNavSelection(navButton, navSelected)
                onClicked: bar.pageRequested(modelData.page)

                background: Rectangle {
                    radius: theme.radiusCard
                    color: navButton.hovered ? theme.accentGhost : theme.accentClear

                    // 选中底色单独一层，只过渡透明度；悬停仍即时变色。
                    Rectangle {
                        objectName: "sidebarSelectionSurface"
                        anchors.fill: parent
                        radius: parent.radius
                        color: theme.accentSoft
                        opacity: navButton.navSelected ? 1 : 0
                        Behavior on opacity {
                            NumberAnimation {
                                duration: theme.motionFast
                                easing.type: Easing.OutCubic
                            }
                        }
                    }
                }

                contentItem: RowLayout {
                    spacing: 10

                    NavIcon {
                        implicitWidth: 18
                        implicitHeight: 18
                        name: navButton.modelData.icon
                        color: navButton.navSelected ? theme.accent : theme.textMuted
                        Layout.alignment: Qt.AlignVCenter
                    }

                    ElidedLabel {
                        Layout.fillWidth: true
                        text: navButton.modelData.label
                        font.pixelSize: theme.fontBody
                        font.weight: navButton.navSelected ? Font.DemiBold : Font.Normal
                        color: navButton.navSelected ? theme.textPrimary : theme.textSecondary
                        elide: Text.ElideRight
                    }

                    KStatusBadge {
                        objectName: navButton.modelData.page === pages.transfers
                                    ? "sidebarTaskBadge" : ""
                        visible: navButton.modelData.page === pages.transfers
                                 && bar.taskCount > 0
                        Layout.alignment: Qt.AlignVCenter
                        text: bar.taskCountText
                        tone: "accent"
                    }
                }
            }
        }

        Item { Layout.fillHeight: true }

        AccountCard {
            Layout.fillWidth: true
            account: bar.backend ? String(bar.backend.account) : ""
            displayName: bar.backend ? String(bar.backend.displayName) : ""
            avatarPath: bar.backend ? String(bar.backend.avatarPath) : ""
            settingsActive: bar.settingsActive
            onSettingsRequested: bar.settingsRequested()
        }
    }

    // 全侧栏只有一根选中竖条，切页时滑动到新入口。
    Rectangle {
        id: selectionBar

        objectName: "sidebarSelectionBar"
        visible: bar.hasNavSelection && bar.selectedNavItem !== null
        enabled: false
        x: theme.gapMd + 1
        y: bar.selectedNavItem
           ? bar.selectedNavItem.mapToItem(
                 bar, 0, (bar.selectedNavItem.height - height) / 2).y
           : 0
        width: 3
        height: 24
        radius: 1.5
        z: 2
        color: theme.accent

        Behavior on y {
            NumberAnimation {
                duration: theme.motionNormal
                easing.type: Easing.OutCubic
            }
        }
    }

    function updateNavSelection(item, selected) {
        if (selected) {
            bar.selectedNavItem = item
            bar.hasNavSelection = true
        } else if (bar.selectedNavItem === item) {
            bar.hasNavSelection = false
        }
    }
}
