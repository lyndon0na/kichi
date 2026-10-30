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
                    title: qsTr("任务结束通知")
                    description: qsTr("下载或离线任务结束时通知。")

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

}
