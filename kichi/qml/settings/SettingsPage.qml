import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "../components"
import "../core"

// 设置对话框的内容：左侧大纲 + 右侧单组设置。
// 所有控件即时生效；文本框失焦或按回车时提交，不再单独放「应用」按钮。
// 底色由宿主（SettingsDialog 的圆角面板）画，这里保持透明。
Item {
    id: page

    property var backend: null

    /// 当前选中的分组，索引对应 sections
    property int currentSection: 0

    readonly property int accountSection: 0
    readonly property int playbackSection: 1
    readonly property int transfersSection: 2
    readonly property int generalSection: 3
    readonly property int aboutSection: 4

    readonly property var sections: [
        { label: qsTr("账号"), icon: "user", component: accountSectionComponent },
        { label: qsTr("播放"), icon: "play", component: playbackSectionComponent },
        { label: qsTr("传输"), icon: "transfer", component: transfersSectionComponent },
        { label: qsTr("通知与后台"), icon: "bell", component: generalSectionComponent },
        { label: qsTr("关于"), icon: "info", component: aboutSectionComponent }
    ]

    /// 退出登录：对话框只负责发信号，确认框在 SignOutDialog，清令牌在 backend
    signal signOutRequested()

    readonly property KTheme theme: KTheme { parent: page }
    readonly property UiMetrics metrics: UiMetrics {}

    /// 内容区左右留白：比主窗口页边距宽一档，窗口里不显得贴边
    readonly property int paneMargin: theme.gapXl

    onCurrentSectionChanged: {
        scroller.contentY = 0
        sectionTransition.restart()
    }

    // 换分组：新内容淡入并轻微上移，和主界面换页同一套节奏。
    // 动画挂在内容上，滚动条与滚动容器不动。
    ParallelAnimation {
        id: sectionTransition

        NumberAnimation {
            target: bodyLoader
            property: "opacity"
            from: 0
            to: 1
            duration: page.theme.motionNormal
            easing.type: Easing.OutCubic
        }

        NumberAnimation {
            target: bodyLoader
            property: "y"
            from: page.theme.motionPageLift
            to: 0
            duration: page.theme.motionNormal
            easing.type: Easing.OutCubic
        }
    }

    RowLayout {
        anchors.fill: parent
        spacing: 0

        SettingsOutline {
            Layout.preferredWidth: 216
            Layout.fillHeight: true
            sections: page.sections
            currentIndex: page.currentSection
            onSectionRequested: function (index) {
                page.selectSection(index)
            }
        }

        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true

            Flickable {
                id: scroller
                objectName: "settingsScroller"
                anchors.fill: parent
                contentWidth: width
                contentHeight: bodyLoader.implicitHeight + page.paneMargin
                clip: true
                boundsBehavior: Flickable.StopAtBounds

                ScrollBar.vertical: KScrollBar {}

                Loader {
                    id: bodyLoader
                    objectName: "settingsBodyLoader"
                    x: page.paneMargin
                    width: Math.max(0, scroller.width - page.paneMargin * 2)
                    sourceComponent: page.sections[page.currentSection].component
                }
            }

            ScrollFirstClickArea {
                id: firstClickArea
                view: scroller
                scrollBarSlot: metrics.scrollBarSlot
                // 设置页没有模型下标；这里只借覆盖层接管惯性中的首击。
                hitTest: function (x, y) {
                    return 0
                }

                onClickedAt: function (index, modifiers, position) {
                    page.activateAt(position.x, position.y)
                }
            }
        }
    }

    Component {
        id: playbackSectionComponent

        SettingsPlayback {
            backend: page.backend
        }
    }

    Component {
        id: transfersSectionComponent

        SettingsTransfers {
            backend: page.backend
        }
    }

    Component {
        id: generalSectionComponent

        SettingsGeneral {
            backend: page.backend
        }
    }

    Component {
        id: accountSectionComponent

        SettingsAccount {
            backend: page.backend
            onSignOutRequested: page.signOutRequested()
        }
    }

    Component {
        id: aboutSectionComponent

        SettingsAbout {
            backend: page.backend
        }
    }

    function selectSection(index) {
        if (index < 0 || index >= page.sections.length || index === page.currentSection)
            return
        page.currentSection = index
    }

    function pointerOver(item, x, y) {
        if (!item || !item.visible)
            return false
        const local = scroller.mapToItem(item, x, y)
        return local.x >= 0 && local.y >= 0
                && local.x < item.width && local.y < item.height
    }

    function activationKind(item) {
        if (item.up && item.down
                && typeof item.increase === "function"
                && typeof item.decrease === "function")
            return "spin"
        if (typeof item.click === "function")
            return "button"
        if (item.popup && typeof item.popup.open === "function")
            return "combo"
        // 整行 MouseArea 盖在控件上方，不能把它的 clicked 信号当成按钮点击。
        if (typeof item.acceptedButtons === "number"
                && typeof item.containsMouse === "boolean")
            return ""
        if (typeof item.clicked === "function")
            return "row"
        if (typeof item.cursorPosition === "number"
                && typeof item.positionAt === "function")
            return "text"
        return ""
    }

    /// 把惯性期间的首击按落点补发给真实控件。按层级取最深的可操作项，
    /// 避免设置行上的整行 MouseArea 抢在按钮、复选框前面。
    function activateAt(x, y) {
        let bestItem = null
        let bestKind = ""
        let bestDepth = -1

        function visit(item, depth) {
            if (!item || !item.visible)
                return
            const local = scroller.mapToItem(item, x, y)
            if (local.x < 0 || local.y < 0
                    || local.x >= item.width || local.y >= item.height)
                return
            if (item.enabled !== false) {
                const kind = page.activationKind(item)
                if (kind.length > 0 && depth >= bestDepth) {
                    bestItem = item
                    bestKind = kind
                    bestDepth = depth
                }
            }
            const children = item.children || []
            for (let childIndex = 0; childIndex < children.length; ++childIndex)
                visit(children[childIndex], depth + 1)
        }

        visit(scroller.contentItem, 0)
        if (!bestItem)
            return
        if (bestKind === "spin") {
            if (pointerOver(bestItem.up, x, y))
                bestItem.increase()
            else if (pointerOver(bestItem.down, x, y))
                bestItem.decrease()
            else
                bestItem.forceActiveFocus()
        } else if (bestKind === "button") {
            bestItem.click()
        } else if (bestKind === "combo") {
            bestItem.popup.open()
        } else if (bestKind === "row") {
            bestItem.clicked()
        } else if (bestKind === "text") {
            bestItem.forceActiveFocus()
            const local = scroller.mapToItem(bestItem, x, y)
            bestItem.cursorPosition = bestItem.positionAt(local.x, local.y)
        }
    }
}
