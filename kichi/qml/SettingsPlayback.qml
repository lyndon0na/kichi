import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 设置页「播放」段：常用项直接展示，播放器命令收进高级项。
ColumnLayout {
    id: playback

    property var backend: null
    property bool advancedOpen: false

    readonly property KTheme theme: KTheme { }

    spacing: 0

    KSettingsGroup {
        title: qsTr("播放")
        Layout.fillWidth: true

        KSettingsRow {
            title: qsTr("默认清晰度")
            description: qsTr("没有对应档位时播放原画。")

            KComboBox {
                id: qualityBox
                objectName: "settingsQuality"
                width: 240
                model: [
                    { "value": "ask", "label": qsTr("每次询问") },
                    { "value": "original", "label": qsTr("原画") },
                    { "value": "transcode:1080P", "label": qsTr("1080P") },
                    { "value": "transcode:720P", "label": qsTr("720P") },
                    { "value": "transcode:480P", "label": qsTr("480P") }
                ]
                textRole: "label"
                valueRole: "value"

                Component.onCompleted: playback.syncFromBackend()
                Connections {
                    target: playback.backend
                    function onChanged() {
                        playback.syncFromBackend()
                    }
                }
                function syncFromBackend() {
                    if (!playback.backend)
                        return
                    var index = qualityBox.indexOfValue(playback.backend.preferredQuality)
                    if (index >= 0 && index !== qualityBox.currentIndex)
                        qualityBox.currentIndex = index
                }
                // onActivated 时不保证 currentValue 已更新，用 index 从 model 里取
                onActivated: function (index) {
                    if (!playback.backend)
                        return
                    playback.backend.setPreferredQuality(qualityBox.model[index].value)
                }
            }
        }

        KSettingsRow {
            divider: true
            title: qsTr("自动加载同目录字幕")
            description: qsTr("同名优先，最多等待 2 秒；仅 mpv 生效。")

            CheckBox {
                checked: playback.backend ? playback.backend.autoSubtitles : true
                Accessible.name: qsTr("自动加载同目录字幕")
                onToggled: function () {
                    if (playback.backend)
                        playback.backend.setAutoSubtitles(checked)
                }
            }
        }

        KSettingsRow {
            objectName: "settingsAdvancedPlayer"
            divider: true
            interactive: true
            title: qsTr("高级播放器设置")
            description: qsTr("自定义命令与参数；留空使用内置默认。")
            onClicked: playback.advancedOpen = !playback.advancedOpen

            NavIcon {
                implicitWidth: 16
                implicitHeight: 16
                name: playback.advancedOpen ? "chevron-down" : "chevron-right"
                color: theme.textMuted
            }
        }

        ColumnLayout {
            objectName: "settingsPlayerAdvancedBody"
            visible: playback.advancedOpen
            Layout.fillWidth: true
            Layout.margins: theme.gapLg
            spacing: theme.gapSm

            RowLayout {
                Layout.fillWidth: true
                spacing: theme.gapMd

                Label {
                    text: qsTr("播放器命令")
                    Layout.preferredWidth: 96
                    font.pixelSize: theme.fontBody
                    color: theme.textPrimary
                }

                KTextField {
                    id: commandField
                    Layout.fillWidth: true
                    selectByMouse: true
                    placeholderText: playback.backend ? playback.backend.playerDefaultCommand : ""
                    text: playback.backend ? playback.backend.playerCommand : ""
                    onEditingFinished: playback.applyPlayer()
                }
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: theme.gapMd

                Label {
                    text: qsTr("附加参数")
                    Layout.preferredWidth: 96
                    font.pixelSize: theme.fontBody
                    color: theme.textPrimary
                }

                KTextField {
                    id: argsField
                    Layout.fillWidth: true
                    selectByMouse: true
                    placeholderText: playback.backend ? playback.backend.playerDefaultArgs : ""
                    text: playback.backend ? playback.backend.playerArgs : ""
                    onEditingFinished: playback.applyPlayer()
                }
            }

            RowLayout {
                Layout.fillWidth: true

                Item { Layout.fillWidth: true }

                KButton {
                    text: qsTr("恢复默认")
                    variant: "ghost"
                    onClicked: playback.resetPlayer()
                }
            }
        }
    }

    function syncFromBackend() {
        if (!playback.backend)
            return
        qualityBox.syncFromBackend()
        if (!commandField.activeFocus)
            commandField.text = playback.backend.playerCommand
        if (!argsField.activeFocus)
            argsField.text = playback.backend.playerArgs
    }

    function applyPlayer() {
        if (!playback.backend)
            return
        playback.backend.setPlayer(commandField.text, argsField.text)
        // 后端可能拒绝非法命令；读回实际生效值，别把失败输入留在框里。
        commandField.text = playback.backend.playerCommand
        argsField.text = playback.backend.playerArgs
    }

    function resetPlayer() {
        if (!playback.backend)
            return
        playback.backend.resetPlayer()
        commandField.text = playback.backend.playerCommand
        argsField.text = playback.backend.playerArgs
    }
}
