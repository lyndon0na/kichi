import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 设置对话框「传输」页：下载目录、下载 / 上传吞吐参数都在这里即时生效。
ColumnLayout {
    id: transfers

    property var backend: null

    readonly property KTheme theme: KTheme { parent: transfers }

    spacing: theme.gapXl

    KSettingsGroup {
        objectName: "settingsDownloadGroup"
        title: qsTr("下载")
        Layout.fillWidth: true

        ColumnLayout {
            Layout.fillWidth: true
            Layout.topMargin: theme.gapSm
            spacing: theme.gapXs

            Label {
                text: qsTr("下载目录")
                font.pixelSize: theme.fontBody
                font.weight: Font.Medium
                color: theme.textPrimary
            }

            Label {
                text: qsTr("留空使用 ~/Downloads；修改后立即生效。")
                font.pixelSize: theme.fontMeta
                color: theme.textMuted
            }

            RowLayout {
                Layout.fillWidth: true
                Layout.topMargin: theme.gapSm
                spacing: theme.gapSm

                KTextField {
                    id: downloadField
                    objectName: "settingsDownloadDir"
                    Layout.fillWidth: true
                    selectByMouse: true
                    placeholderText: qsTr("留空使用 ~/Downloads")
                    text: transfers.backend ? transfers.backend.downloadDir : ""
                    onEditingFinished: transfers.applyDownloadDir()
                }

                KButton {
                    objectName: "settingsChooseDownloadDir"
                    text: qsTr("选择目录")
                    variant: "secondary"
                    iconName: "folder"
                    onClicked: {
                        if (transfers.backend)
                            transfers.backend.chooseDownloadDir()
                    }
                }

                KButton {
                    objectName: "settingsOpenDownloadDir"
                    text: qsTr("打开目录")
                    variant: "ghost"
                    onClicked: {
                        if (transfers.backend)
                            transfers.backend.openDownloadDir()
                    }
                }
            }
        }

        KSettingsRow {
            title: qsTr("同时下载任务数")
            description: qsTr("跨任务与文件夹内共用，范围 1–8。")

            SpinBox {
                id: parallelBox
                objectName: "settingsDownloadParallel"
                from: 1
                to: 8
                stepSize: 1
                editable: true
                value: transfers.backend ? transfers.backend.downloadParallel : 3
                Accessible.name: qsTr("同时下载任务数")
                onValueModified: {
                    if (transfers.backend)
                        transfers.backend.setDownloadParallel(value)
                }
            }
        }

        KSettingsRow {
            title: qsTr("单文件连接数")
            description: qsTr("大文件的分片连接数；1 表示不分片，范围 1–8。")

            SpinBox {
                id: connectionsBox
                objectName: "settingsDownloadConnections"
                from: 1
                to: 8
                stepSize: 1
                editable: true
                value: transfers.backend ? transfers.backend.downloadConnections : 4
                Accessible.name: qsTr("单文件连接数")
                onValueModified: {
                    if (transfers.backend)
                        transfers.backend.setDownloadConnections(value)
                }
            }
        }

        KSettingsRow {
            title: qsTr("全局限速")
            description: qsTr("所有下载连接共享此上限，0 表示不限速。")

            RowLayout {
                spacing: theme.gapXs

                SpinBox {
                    id: rateBox
                    objectName: "settingsDownloadRate"
                    from: 0
                    to: 1024
                    stepSize: 1
                    editable: true
                    implicitWidth: 96
                    value: transfers.backend ? transfers.backend.downloadRateMib : 0
                    Accessible.name: qsTr("全局限速")
                    onValueModified: {
                        if (transfers.backend)
                            transfers.backend.setDownloadRateMib(value)
                    }
                }

                Label {
                    text: qsTr("MiB/s")
                    color: theme.textMuted
                    font.pixelSize: theme.fontMeta
                }
            }
        }

        KSettingsRow {
            title: qsTr("正常退出后自动续跑")
            description: qsTr("下次恢复登录态时继续未完成任务；暂停、失败、取消和崩溃不自动续跑。")

            Switch {
                objectName: "settingsAutoResumeDownloads"
                checked: transfers.backend ? transfers.backend.autoResumeDownloads : true
                Accessible.name: qsTr("正常退出后自动续跑")
                onToggled: function () {
                    if (transfers.backend)
                        transfers.backend.setAutoResumeDownloads(checked)
                }
            }
        }
    }

    KSettingsGroup {
        objectName: "settingsUploadGroup"
        title: qsTr("上传")
        Layout.fillWidth: true

        KSettingsRow {
            title: qsTr("同时上传任务数")
            description: qsTr("单文件任务跨批共用；文件夹批次独占启动、批内共用，范围 1–8。")

            SpinBox {
                id: uploadParallelBox
                objectName: "settingsUploadParallel"
                from: 1
                to: 8
                stepSize: 1
                editable: true
                value: transfers.backend ? transfers.backend.uploadParallel : 2
                Accessible.name: qsTr("同时上传任务数")
                onValueModified: {
                    if (transfers.backend)
                        transfers.backend.setUploadParallel(value)
                }
            }
        }

        KSettingsRow {
            title: qsTr("全局限速")
            description: qsTr("所有上传连接共享此上限，0 表示不限速；只限制实际上传的字节。")

            RowLayout {
                spacing: theme.gapXs

                SpinBox {
                    id: uploadRateBox
                    objectName: "settingsUploadRate"
                    from: 0
                    to: 1024
                    stepSize: 1
                    editable: true
                    implicitWidth: 96
                    value: transfers.backend ? transfers.backend.uploadRateMib : 0
                    Accessible.name: qsTr("上传全局限速")
                    onValueModified: {
                        if (transfers.backend)
                            transfers.backend.setUploadRateMib(value)
                    }
                }

                Label {
                    text: qsTr("MiB/s")
                    color: theme.textMuted
                    font.pixelSize: theme.fontMeta
                }
            }
        }

        KSettingsRow {
            title: qsTr("正常退出后自动续传上传")
            description: qsTr("默认关闭。开启后只恢复正常退出时仍在进行的上传；暂停、失败、取消和崩溃不恢复。有断点的大文件接着传，5 MiB 以内的小文件会从头重传。")

            Switch {
                objectName: "settingsAutoResumeUploads"
                checked: transfers.backend ? transfers.backend.autoResumeUploads : false
                Accessible.name: qsTr("正常退出后自动续传上传")
                onToggled: function () {
                    if (transfers.backend)
                        transfers.backend.setAutoResumeUploads(checked)
                }
            }
        }
    }

    Connections {
        target: transfers.backend
        function onChanged() {
            transfers.syncFromBackend()
        }
    }

    function syncFromBackend() {
        if (!transfers.backend)
            return
        if (!downloadField.activeFocus)
            downloadField.text = transfers.backend.downloadDir
        if (!parallelBox.activeFocus)
            parallelBox.value = transfers.backend.downloadParallel
        if (!connectionsBox.activeFocus)
            connectionsBox.value = transfers.backend.downloadConnections
        if (!rateBox.activeFocus)
            rateBox.value = transfers.backend.downloadRateMib
        if (!uploadParallelBox.activeFocus)
            uploadParallelBox.value = transfers.backend.uploadParallel
        if (!uploadRateBox.activeFocus)
            uploadRateBox.value = transfers.backend.uploadRateMib
    }

    function applyDownloadDir() {
        if (!transfers.backend)
            return
        transfers.backend.setDownloadDir(downloadField.text)
        // 后端可能拒绝非法路径；读回实际生效值。
        downloadField.text = transfers.backend.downloadDir
    }
}
