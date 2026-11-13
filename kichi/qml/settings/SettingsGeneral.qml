import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "../components"
import "../core"

// 设置对话框「通知与后台」页：本地索引、任务通知和关窗行为。
ColumnLayout {
    id: general

    property var backend: null

    /// 重建索引需要确认；确认框由 SettingsPage 承载，避免弹窗被布局托管。
    signal rebuildIndexRequested()

    readonly property KTheme theme: KTheme { parent: general }
    readonly property bool indexEnabled: backend !== null
                                         && backend.localIndexEnabled === true
    readonly property bool indexReady: backend !== null
                                       && backend.localIndexReady === true
    readonly property bool indexBuilding: backend !== null
                                         && backend.indexBuilding === true
    readonly property int indexScanned: backend !== null
                                        && backend.indexBuildScanned !== undefined
                                        ? backend.indexBuildScanned : 0
    readonly property string indexBuiltAt: backend !== null
                                           && backend.localIndexBuiltAt !== undefined
                                           ? backend.localIndexBuiltAt : ""

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
            description: {
                const base = qsTr("在本机保存目录与页面快照，用于重启秒开、路径定位与全盘搜索。输入关键词时只筛当前文件夹，按回车或点搜索按钮才搜整个网盘；全盘搜索会同时建立索引，就绪后直接读本地库，不再发出请求。关闭时不后台预建、不读写本地库；下次全盘搜索会自动重新开启并建立索引。")
                if (!general.indexEnabled)
                    return qsTr("未启用。%1").arg(base)
                if (general.indexBuilding)
                    return qsTr("正在建立本地索引（已扫描 %1 条）……%2")
                            .arg(general.indexScanned).arg(base)
                if (general.indexReady)
                    return qsTr("已建立 · 更新于 %1。%2")
                            .arg(general.indexBuiltAt).arg(base)
                return qsTr("尚未建立。%1").arg(base)
            }

            RowLayout {
                spacing: general.theme.gapSm

                KButton {
                    objectName: "settingsBuildIndex"
                    visible: general.indexEnabled && !general.indexBuilding
                    text: general.indexReady ? qsTr("重建索引") : qsTr("建立索引")
                    variant: general.indexReady ? "ghost" : "secondary"
                    onClicked: {
                        if (!general.backend)
                            return
                        if (general.indexReady)
                            general.rebuildIndexRequested()
                        else
                            general.backend.buildLocalIndex(false)
                    }
                }

                Switch {
                    objectName: "settingsLocalIndex"
                    checked: general.indexEnabled
                    Accessible.name: qsTr("本地目录索引")
                    onToggled: function () {
                        if (general.backend)
                            general.backend.setLocalIndexEnabled(checked)
                    }
                }
            }
        }
    }

}
