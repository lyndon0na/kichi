import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 设置页「下载」段：目录与传输吞吐参数都在这里即时生效。
ColumnLayout {
    id: downloads

    property var backend: null

    readonly property KTheme theme: KTheme { parent: downloads }

    spacing: theme.gapXl

    KSettingsGroup {
        objectName: "settingsDownloadGroup"
        title: qsTr("下载")
        Layout.fillWidth: true

        ColumnLayout {
            Layout.fillWidth: true
            Layout.margins: theme.gapLg
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
                    text: downloads.backend ? downloads.backend.downloadDir : ""
                    onEditingFinished: downloads.applyDownloadDir()
                }

                KButton {
                    objectName: "settingsChooseDownloadDir"
                    text: qsTr("选择目录")
                    variant: "secondary"
                    iconName: "folder"
                    onClicked: {
                        if (downloads.backend)
                            downloads.backend.chooseDownloadDir()
                    }
                }

                KButton {
                    objectName: "settingsOpenDownloadDir"
                    text: qsTr("打开目录")
                    variant: "ghost"
                    onClicked: {
                        if (downloads.backend)
                            downloads.backend.openDownloadDir()
                    }
                }
            }
        }

        KSettingsRow {
            divider: true
            title: qsTr("同时下载任务数")
            description: qsTr("跨任务与文件夹内共用，范围 1–8。")

            SpinBox {
                id: parallelBox
                objectName: "settingsDownloadParallel"
                from: 1
                to: 8
                stepSize: 1
                editable: true
                value: downloads.backend ? downloads.backend.downloadParallel : 3
                Accessible.name: qsTr("同时下载任务数")
                onValueModified: {
                    if (downloads.backend)
                        downloads.backend.setDownloadParallel(value)
                }
            }
        }

        KSettingsRow {
            divider: true
            title: qsTr("单文件连接数")
            description: qsTr("大文件的分片连接数；1 表示不分片，范围 1–8。")

            SpinBox {
                id: connectionsBox
                objectName: "settingsDownloadConnections"
                from: 1
                to: 8
                stepSize: 1
                editable: true
                value: downloads.backend ? downloads.backend.downloadConnections : 4
                Accessible.name: qsTr("单文件连接数")
                onValueModified: {
                    if (downloads.backend)
                        downloads.backend.setDownloadConnections(value)
                }
            }
        }

        KSettingsRow {
            divider: true
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
                    value: downloads.backend ? downloads.backend.downloadRateMib : 0
                    Accessible.name: qsTr("全局限速")
                    onValueModified: {
                        if (downloads.backend)
                            downloads.backend.setDownloadRateMib(value)
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
            divider: true
            title: qsTr("正常退出后自动续跑")
            description: qsTr("下次恢复登录态时继续未完成任务；暂停、失败、取消和崩溃不自动续跑。")

            CheckBox {
                objectName: "settingsAutoResumeDownloads"
                checked: downloads.backend ? downloads.backend.autoResumeDownloads : true
                Accessible.name: qsTr("正常退出后自动续跑")
                onToggled: function () {
                    if (downloads.backend)
                        downloads.backend.setAutoResumeDownloads(checked)
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
                value: downloads.backend ? downloads.backend.uploadParallel : 2
                Accessible.name: qsTr("同时上传任务数")
                onValueModified: {
                    if (downloads.backend)
                        downloads.backend.setUploadParallel(value)
                }
            }
        }

        KSettingsRow {
            divider: true
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
                    value: downloads.backend ? downloads.backend.uploadRateMib : 0
                    Accessible.name: qsTr("上传全局限速")
                    onValueModified: {
                        if (downloads.backend)
                            downloads.backend.setUploadRateMib(value)
                    }
                }

                Label {
                    text: qsTr("MiB/s")
                    color: theme.textMuted
                    font.pixelSize: theme.fontMeta
                }
            }
        }
    }

    Connections {
        target: downloads.backend
        function onChanged() {
            downloads.syncFromBackend()
        }
    }

    function syncFromBackend() {
        if (!downloads.backend)
            return
        if (!downloadField.activeFocus)
            downloadField.text = downloads.backend.downloadDir
        if (!parallelBox.activeFocus)
            parallelBox.value = downloads.backend.downloadParallel
        if (!connectionsBox.activeFocus)
            connectionsBox.value = downloads.backend.downloadConnections
        if (!rateBox.activeFocus)
            rateBox.value = downloads.backend.downloadRateMib
        if (!uploadParallelBox.activeFocus)
            uploadParallelBox.value = downloads.backend.uploadParallel
        if (!uploadRateBox.activeFocus)
            uploadRateBox.value = downloads.backend.uploadRateMib
    }

    function applyDownloadDir() {
        if (!downloads.backend)
            return
        downloads.backend.setDownloadDir(downloadField.text)
        // 后端可能拒绝非法路径；读回实际生效值。
        downloadField.text = downloads.backend.downloadDir
    }
}
