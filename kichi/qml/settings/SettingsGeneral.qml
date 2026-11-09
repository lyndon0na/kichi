import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "../components"
import "../core"

// 设置对话框「通知与后台」页：本地索引、任务通知和关窗行为。
ColumnLayout {
    id: general

    property var backend: null

    readonly property KTheme theme: KTheme { parent: general }

    spacing: theme.gapXl

    KSettingsGroup {
        title: qsTr("通知")
        Layout.fillWidth: true

        KSettingsRow {
            title: qsTr("任务结束通知")
            description: qsTr("下载、上传或离线任务结束时通知。")

            Switch {
                objectName: "settingsNotify"
                checked: general.backend ? general.backend.notifyEnabled : true
                Accessible.name: qsTr("任务结束通知")
                onToggled: function () {
                    if (general.backend)
                        general.backend.setNotifyEnabled(checked)
                }
            }
        }
    }

    KSettingsGroup {
        title: qsTr("运行方式")
        Layout.fillWidth: true

        KSettingsRow {
            title: qsTr("关闭窗口时最小化到托盘")
            description: qsTr("任务继续运行；托盘不可用时保持关闭。")

            Switch {
                objectName: "settingsCloseToTray"
                checked: general.backend ? general.backend.closeToTray : false
                Accessible.name: qsTr("关闭窗口时最小化到托盘")
                onToggled: function () {
                    if (general.backend)
                        general.backend.setCloseToTray(checked)
                }
            }
        }
    }

    KSettingsGroup {
        title: qsTr("本地数据")
        Layout.fillWidth: true

        KSettingsRow {
            title: qsTr("本地目录索引")
            description: qsTr("在本机保存目录与页面快照，用于输入即搜、重启秒开与路径定位。关闭时不读写本地库，已保存的库文件保留。")

            Switch {
                objectName: "settingsLocalIndex"
                checked: general.backend ? general.backend.localIndexEnabled : false
                Accessible.name: qsTr("本地目录索引")
                onToggled: function () {
                    if (general.backend)
                        general.backend.setLocalIndexEnabled(checked)
                }
            }
        }
    }
}
