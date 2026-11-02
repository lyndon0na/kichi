import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 登录后的固定导航栏。账号信息放在底部，内容区只负责当前页面。
// 退出登录在设置页「账号」一节（低频、不可逆，别放回侧栏）。
//
// 视觉：侧栏底色比内容区低半档，导航项是圆角卡片；选中态用「淡强调色底 +
// 3px 竖条 + 强调色图标」，不铺满饱和色块（与文件列表的选中语言一致）。
Rectangle {
    id: bar

    property var backend: null
    property int currentIndex: 0

    signal pageRequested(int pageIndex)
    signal settingsRequested()

    KTheme { id: theme }
    PageIndex { id: pages }

    readonly property int unfinishedTaskCount: {
        if (!bar.backend)
            return 0
        const tasks = bar.backend.tasks || []
        let count = 0
        for (let i = 0; i < tasks.length; ++i) {
            if (tasks[i].settled !== true)
                ++count
        }
        return count
    }

    color: theme.surfaceSheet

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: theme.gapMd
        spacing: theme.gapXs

        Repeater {
            // 显示顺序与 main.qml 的页面索引解耦；page 始终对应实际页面。
            model: [
                { "page": pages.files, "label": qsTr("我的文件"), "icon": "folder" },
                { "page": pages.recentSaved, "label": qsTr("最近保存"), "icon": "clock" },
                { "page": pages.playbackHistory, "label": qsTr("播放历史"), "icon": "history" },
                { "page": pages.starred, "label": qsTr("已加星标"), "icon": "star" },
                { "page": pages.tasks, "label": qsTr("离线任务"), "icon": "download" },
                { "page": pages.shares, "label": qsTr("分享"), "icon": "share" },
                { "page": pages.trash, "label": qsTr("回收站"), "icon": "trash" }
            ]

            delegate: Button {
                id: navButton
                required property var modelData

                // Button 的 checkable 会在点击时自行改写 checked，并永久切断与当前页的
                // 绑定；选中态只认 currentIndex，避免连点当前入口后高亮消失。
                readonly property bool navSelected: bar.currentIndex === modelData.page

                objectName: "sidebarNav_" + modelData.page
                Layout.fillWidth: true
                Layout.preferredHeight: 40
                padding: 0
                leftPadding: 12
                rightPadding: 12
                onClicked: bar.pageRequested(modelData.page)

                background: Rectangle {
                    radius: theme.radiusCard
                    color: navButton.navSelected ? theme.accentSoft
                         : navButton.hovered ? theme.accentGhost : theme.accentClear


                    // 选中竖条：和文件列表的选中语言一致
                    Rectangle {
                        visible: navButton.navSelected
                        width: 3
                        height: parent.height - 16
                        radius: 1.5
                        anchors.left: parent.left
                        anchors.leftMargin: 1
                        anchors.verticalCenter: parent.verticalCenter
                        color: theme.accent
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

                    // 离线任务：未结束条数徽标（0 时隐藏）。
                    KStatusBadge {
                        objectName: navButton.modelData.page === pages.tasks
                                    ? "sidebarTaskBadge" : ""
                        readonly property int count: navButton.modelData.page === pages.tasks
                                                    ? bar.unfinishedTaskCount : 0
                        readonly property bool countVisible: count > 0
                        Layout.alignment: Qt.AlignVCenter
                        visible: countVisible
                        text: count > 99 ? "99+" : String(count)
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
            quotaUsed: bar.backend ? bar.backend.quotaUsed : ""
            quotaLimit: bar.backend ? bar.backend.quotaLimit : ""
            quotaLoading: bar.backend ? bar.backend.quotaLoading : false
            quotaRatio: bar.backend ? bar.backend.quotaRatio : -1
            vip: bar.backend ? bar.backend.quotaVip : false
            settingsActive: bar.currentIndex === pages.settings
            onSettingsRequested: bar.settingsRequested()
        }
    }
}
