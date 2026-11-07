import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "../components"
import "../core"

// 设置对话框「账号」页：当月传输额度、令牌存储位置与退出登录。
// 只发信号，确认框在 SignOutDialog，清令牌在 backend。
ColumnLayout {
    id: account

    property var backend: null

    signal signOutRequested()

    readonly property KTheme theme: KTheme { parent: account }

    spacing: theme.gapXl

    // 服务端没下发时先显示加载态，加载完仍为空才算未知
    function quotaText(value) {
        if (!account.backend)
            return ""
        if (value.length > 0)
            return value
        return account.backend.quotaLoading ? qsTr("获取中…") : qsTr("未知")
    }

    // 一行额度：右侧是「已用 X / Y」+ 占用条；没有上限时不画条
    component QuotaMeter: ColumnLayout {
        id: meter

        property string meterName: ""
        property string value: ""
        property real ratio: -1

        readonly property KTheme theme: KTheme { parent: meter }

        implicitWidth: 200
        spacing: theme.gapXs

        Label {
            objectName: meter.meterName
            Layout.fillWidth: true
            text: meter.value
            horizontalAlignment: Text.AlignRight
            font.pixelSize: theme.fontMeta
            color: theme.textSecondary
        }

        KProgressBar {
            objectName: meter.meterName + "Bar"
            Layout.fillWidth: true
            from: 0
            to: 1
            // 用了但比例极小时至少留 1%（2px）的可见填充，避免条看着像空的
            value: meter.ratio <= 0 ? 0 : Math.max(meter.ratio, 0.01)
            // 取不到上限时保留位置但不画条，避免数据到达时整页高度跳动
            opacity: meter.ratio >= 0 ? 1 : 0
        }
    }

    KSettingsGroup {
        title: qsTr("传输配额")
        Layout.fillWidth: true

        KSettingsRow {
            title: qsTr("云下载")
            description: qsTr("离线保存到网盘的当月流量。")

            QuotaMeter {
                meterName: "settingsTransferCloud"
                value: account.backend ? account.quotaText(account.backend.quotaTransferCloud) : ""
                ratio: account.backend ? account.backend.quotaTransferCloudRatio : -1
            }
        }

        KSettingsRow {
            title: qsTr("下行")
            description: qsTr("播放、在线查看与下载的当月流量。")

            QuotaMeter {
                meterName: "settingsTransferDownload"
                value: account.backend ? account.quotaText(account.backend.quotaTransferDownload) : ""
                ratio: account.backend ? account.backend.quotaTransferDownloadRatio : -1
            }
        }

        KSettingsRow {
            title: qsTr("上传")
            description: qsTr("当月上传的流量。")

            QuotaMeter {
                meterName: "settingsTransferUpload"
                value: account.backend ? account.quotaText(account.backend.quotaTransferUpload) : ""
                ratio: account.backend ? account.backend.quotaTransferUploadRatio : -1
            }
        }

        KSettingsRow {
            title: qsTr("重置时间")

            Label {
                objectName: "settingsTransferReset"
                text: account.backend ? account.backend.quotaTransferReset : ""
                font.pixelSize: theme.fontMeta
                color: theme.textSecondary
            }
        }
    }

    KSettingsGroup {
        title: qsTr("账号")
        Layout.fillWidth: true

        KSettingsRow {
            title: qsTr("令牌存储")

            Label {
                text: account.backend ? account.backend.storageBackend : ""
                font.pixelSize: theme.fontMeta
                color: theme.textSecondary
            }
        }

        KSettingsRow {
            danger: true
            title: qsTr("退出登录")
            description: qsTr("清除本机令牌，下次需重新登录。")

            KButton {
                objectName: "settingsSignOut"
                text: qsTr("退出登录")
                variant: "dangerOutline"
                onClicked: account.signOutRequested()
            }
        }
    }
}
