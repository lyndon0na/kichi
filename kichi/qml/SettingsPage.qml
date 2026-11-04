import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 设置页：分组卡片 + 统一设置行。
// 所有控件即时生效；文本框失焦或按回车时提交，不再单独放「应用」按钮。
Item {
    id: page

    property var backend: null

    /// 退出登录：设置页只负责发信号，确认框与清令牌在 main.qml / backend 那边
    signal signOutRequested()

    UiMetrics {
        id: metrics
    }

    KTheme { id: theme }

    Flickable {
        id: scroller
        objectName: "settingsScroller"
        anchors.fill: parent
        anchors.leftMargin: metrics.pageMargin
        anchors.topMargin: metrics.pageMargin
        anchors.bottomMargin: metrics.pageMargin
        clip: true
        contentWidth: width
        contentHeight: body.implicitHeight
        boundsBehavior: Flickable.StopAtBounds

        ScrollBar.vertical: KScrollBar {}

        ColumnLayout {
            id: body
            width: scroller.width - metrics.scrollBarSlot
            spacing: 0

            SettingsPlayback {
                objectName: "settingsPlayback"
                Layout.fillWidth: true
                backend: page.backend
            }

            SettingsDownloads {
                objectName: "settingsDownloads"
                Layout.fillWidth: true
                Layout.topMargin: theme.gapXl
                backend: page.backend
            }

            KSettingsGroup {
                title: qsTr("通知与后台")
                Layout.fillWidth: true
                Layout.topMargin: theme.gapXl

                KSettingsRow {
                    title: qsTr("本地目录索引")
                    description: qsTr("在本机保存目录与页面快照，用于输入即搜、重启秒开与路径定位。关闭时不读写本地库，已保存的库文件保留。")

                    CheckBox {
                        objectName: "settingsLocalIndex"
                        checked: page.backend ? page.backend.localIndexEnabled : false
                        Accessible.name: qsTr("本地目录索引")
                        onToggled: function () {
                            if (page.backend)
                                page.backend.setLocalIndexEnabled(checked)
                        }
                    }
                }

                KSettingsRow {
                    divider: true
                    title: qsTr("任务结束通知")
                    description: qsTr("下载、上传或离线任务结束时通知。")

                    CheckBox {
                        objectName: "settingsNotify"
                        checked: page.backend ? page.backend.notifyEnabled : true
                        Accessible.name: qsTr("任务结束通知")
                        onToggled: function () {
                            if (page.backend)
                                page.backend.setNotifyEnabled(checked)
                        }
                    }
                }

                KSettingsRow {
                    divider: true
                    title: qsTr("关闭窗口时最小化到托盘")
                    description: qsTr("任务继续运行；托盘不可用时保持关闭。")

                    CheckBox {
                        objectName: "settingsCloseToTray"
                        checked: page.backend ? page.backend.closeToTray : false
                        Accessible.name: qsTr("关闭窗口时最小化到托盘")
                        onToggled: function () {
                            if (page.backend)
                                page.backend.setCloseToTray(checked)
                        }
                    }
                }
            }

            KSettingsGroup {
                title: qsTr("账号与安全")
                Layout.fillWidth: true
                Layout.topMargin: theme.gapXl

                KSettingsRow {
                    title: qsTr("令牌存储")

                    Label {
                        text: page.backend ? page.backend.storageBackend : ""
                        font.pixelSize: theme.fontMeta
                        color: theme.textSecondary
                    }
                }

                KSettingsRow {
                    divider: true
                    danger: true
                    title: qsTr("退出登录")
                    description: qsTr("清除本机令牌，下次需重新登录。")

                    KButton {
                        objectName: "settingsSignOut"
                        text: qsTr("退出登录")
                        variant: "dangerOutline"
                        onClicked: page.signOutRequested()
                    }
                }
            }

            SettingsAbout {
                Layout.fillWidth: true
                Layout.topMargin: theme.gapXl
                backend: page.backend
            }

            Item { Layout.fillHeight: true }
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
