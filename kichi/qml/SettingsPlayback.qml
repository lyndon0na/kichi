import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 设置页「播放」段：常用项直接展示，播放器命令收进高级项。
ColumnLayout {
    id: playback

    property var backend: null
    property bool advancedOpen: false

    readonly property KTheme theme: KTheme { parent: playback }

    readonly property var subTextColors: [
        { "color": "#FFFFFFFF", "name": qsTr("白色"), "darkText": true },
        { "color": "#FFFFFF00", "name": qsTr("黄色"), "darkText": true },
        { "color": "#FF00FFFF", "name": qsTr("青色"), "darkText": true },
        { "color": "#FF9BE37F", "name": qsTr("绿色"), "darkText": true },
        { "color": "#FFFF9EC9", "name": qsTr("粉色"), "darkText": true },
        { "color": "#FFFFA94D", "name": qsTr("橙色"), "darkText": true }
    ]
    readonly property var subOutlineColors: [
        { "color": "#FF000000", "name": qsTr("黑色"), "darkText": false },
        { "color": "#FFFFFFFF", "name": qsTr("白色"), "darkText": true },
        { "color": "#FF404040", "name": qsTr("深灰"), "darkText": false },
        { "color": "#FF1A2A4A", "name": qsTr("深蓝"), "darkText": false },
        { "color": "#FF4A1A1A", "name": qsTr("深红"), "darkText": false }
    ]

    spacing: 0

    KSettingsGroup {
        title: qsTr("播放")
        Layout.fillWidth: true

        KSettingsRow {
            title: qsTr("播放器")
            description: qsTr("在独立的 kichi 播放窗口里播放，主窗口可同时使用；支持暂停、拖动进度与字幕；也可改为外部 mpv / VLC。外部播放器需系统已安装，缺少时会自动改用另一个。")

            KComboBox {
                id: playerBackendBox
                objectName: "settingsPlayerBackend"
                width: 240
                model: [
                    { "value": "embedded", "label": qsTr("内嵌（默认）") },
                    { "value": "mpv", "label": qsTr("mpv") },
                    { "value": "vlc", "label": qsTr("VLC") },
                    { "value": "custom", "label": qsTr("自定义") }
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
                    var index = playerBackendBox.indexOfValue(playback.backend.playerBackend)
                    if (index >= 0 && index !== playerBackendBox.currentIndex)
                        playerBackendBox.currentIndex = index
                }
                onActivated: function (index) {
                    if (!playback.backend)
                        return
                    var value = playerBackendBox.model[index].value
                    playback.backend.setPlayerBackend(value)
                    if (value === "custom")
                        playback.advancedOpen = true
                }
            }
        }

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
            title: qsTr("自动加载同目录字幕")
            description: qsTr("同名优先，最多等待 2 秒；内嵌 / mpv 生效。")

            Switch {
                checked: playback.backend ? playback.backend.autoSubtitles : true
                Accessible.name: qsTr("自动加载同目录字幕")
                onToggled: function () {
                    if (playback.backend)
                        playback.backend.setAutoSubtitles(checked)
                }
            }
        }

        KSettingsRow {
            title: qsTr("按键交给 mpv")
            description: qsTr("播放窗口把键盘交给 mpv：空格暂停、方向键快进或调音量、m 静音等原生键位生效。Esc（收浮层 / 退全屏 / 关窗）和 F（全屏）始终由 kichi 处理；菜单或样式面板打开时键盘归浮层。关闭时使用 kichi 的快捷键，按键不下发。")

            Switch {
                id: mpvKeysSwitch
                objectName: "settingsPlayerMpvKeys"
                checked: playback.backend ? playback.backend.embedMpvKeys : false
                Accessible.name: qsTr("按键交给 mpv")
                onToggled: function () {
                    if (playback.backend)
                        playback.backend.setMpvKeys(checked)
                }
            }
        }

        KSettingsRow {
            objectName: "settingsAdvancedPlayer"
            interactive: true
            title: qsTr("自定义播放器命令")
            description: qsTr("仅「自定义」模式使用；编辑后会自动切到自定义。")
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
            Layout.topMargin: theme.gapMd
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
                    objectName: "settingsPlayerCommand"
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
                    objectName: "settingsPlayerArgs"
                    Layout.fillWidth: true
                    selectByMouse: true
                    placeholderText: playback.backend ? playback.backend.playerDefaultArgs : ""
                    text: playback.backend ? playback.backend.playerArgs : ""
                    onEditingFinished: playback.applyPlayer()
                }
            }

            Label {
                objectName: "settingsPlayerArgsHint"
                Layout.fillWidth: true
                text: qsTr("非 mpv/VLC 播放器：占位符可放在命令或附加参数里；附加参数留空且命令里没有输入占位符时，默认附加参数是 {playlist}（直接传临时 M3U 路径）。需要 --playlist= 形式时自己写 --playlist={playlist}。{title} 是文件名，{url} 是签名直链；只使用 {url} 时不创建 M3U，且直链会出现在本机进程命令行中。Flatpak 版 mpv 可填 flatpak run --die-with-parent io.mpv.Mpv。")
                font.pixelSize: theme.fontCaption
                color: theme.textMuted
                wrapMode: Text.WordWrap
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

    KSettingsGroup {
        title: qsTr("字幕样式")
        Layout.fillWidth: true

        KSettingsRow {
            title: qsTr("字号")
            description: qsTr("内嵌播放时立即生效。")

            SpinBox {
                id: subFontBox
                objectName: "settingsSubtitleFontSize"
                from: 16
                to: 100
                stepSize: 1
                editable: true
                value: playback.backend ? playback.backend.embedSubFontSize : 55
                Accessible.name: qsTr("字幕字号")
                onValueModified: {
                    if (playback.backend)
                        playback.backend.setSubtitleFontSize(value)
                }
            }
        }

        KSettingsRow {
            title: qsTr("位置")
            description: qsTr("0 = 贴顶部，100 = 底部默认位置，150 = 更靠下。")

            SpinBox {
                id: subPosBox
                objectName: "settingsSubtitlePosition"
                from: 0
                to: 150
                stepSize: 1
                editable: true
                value: playback.backend ? playback.backend.embedSubPos : 100
                Accessible.name: qsTr("字幕位置")
                onValueModified: {
                    if (playback.backend)
                        playback.backend.setSubtitlePosition(value)
                }
            }
        }

        KSettingsRow {
            title: qsTr("文字颜色")
            description: qsTr("播放窗口的字幕菜单里也能随时调整。")

            KColorSwatchRow {
                objectName: "settingsSubtitleTextColor"
                colors: playback.subTextColors
                current: playback.backend ? playback.backend.embedSubColor : "#FFFFFFFF"
                selectionColor: theme.accent
                onColorPicked: function (color) {
                    if (playback.backend)
                        playback.backend.setSubtitleColor(color)
                }
            }
        }

        KSettingsRow {
            title: qsTr("描边粗细")
            description: qsTr("0 表示不描边。")

            SpinBox {
                id: subOutlineBox
                objectName: "settingsSubtitleOutlineSize"
                from: 0
                to: 8
                stepSize: 1
                editable: true
                value: playback.backend ? playback.backend.embedSubOutlineSize : 2
                Accessible.name: qsTr("字幕描边粗细")
                onValueModified: {
                    if (playback.backend)
                        playback.backend.setSubtitleOutlineSize(value)
                }
            }
        }

        KSettingsRow {
            title: qsTr("描边颜色")
            description: qsTr("深色描边在亮画面上更清楚。")

            KColorSwatchRow {
                objectName: "settingsSubtitleOutlineColor"
                colors: playback.subOutlineColors
                current: playback.backend ? playback.backend.embedSubOutlineColor
                                          : "#FF000000"
                selectionColor: theme.accent
                onColorPicked: function (color) {
                    if (playback.backend)
                        playback.backend.setSubtitleOutlineColor(color)
                }
            }
        }
    }

    function syncFromBackend() {
        if (!playback.backend)
            return
        playerBackendBox.syncFromBackend()
        qualityBox.syncFromBackend()
        if (!commandField.activeFocus)
            commandField.text = playback.backend.playerCommand
        if (!argsField.activeFocus)
            argsField.text = playback.backend.playerArgs
        if (!subFontBox.activeFocus)
            subFontBox.value = playback.backend.embedSubFontSize
        if (!subPosBox.activeFocus)
            subPosBox.value = playback.backend.embedSubPos
        if (!subOutlineBox.activeFocus)
            subOutlineBox.value = playback.backend.embedSubOutlineSize
        mpvKeysSwitch.checked = playback.backend.embedMpvKeys
    }

    // 播放窗口的字幕样式调整会改后端属性；设置页开着时同步到控件。
    Connections {
        target: playback.backend
        function onChanged() { playback.syncFromBackend() }
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
