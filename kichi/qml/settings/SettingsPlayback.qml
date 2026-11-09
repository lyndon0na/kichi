import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "../components"
import "../core"

// 设置页「播放」段：常用项直接展示，播放器命令收进高级项。
ColumnLayout {
    id: playback

    property var backend: null
    property bool advancedOpen: false
    /// Flatpak 沙箱里只提供内嵌播放器：外部播放器选项与自定义命令整体隐藏。
    readonly property bool flatpak: playback.backend ? playback.backend.flatpak : false

    readonly property KTheme theme: KTheme { parent: playback }

    spacing: theme.gapXl

    KSettingsGroup {
        title: qsTr("常用")
        Layout.fillWidth: true

        KSettingsRow {
            objectName: "settingsPlayerRow"
            visible: !playback.flatpak
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
            objectName: "settingsPlayerFlatpakOnly"
            visible: playback.flatpak
            title: qsTr("播放器")
            description: qsTr("Flatpak 版只使用内嵌播放器；外部 mpv / VLC 与自定义命令在沙箱内不可用。需要外部播放器时请使用 AppImage 或自行构建。")
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
            objectName: "settingsAutoPlayNext"
            title: qsTr("播完自动播放下一集")
            description: qsTr("同一文件夹内的视频 / 音频按列表顺序连播；内嵌 / mpv 生效，VLC 尽力。片尾前会预取下一集。")

            Switch {
                objectName: "settingsAutoPlayNextToggle"
                checked: playback.backend ? playback.backend.autoPlayNext : true
                Accessible.name: qsTr("播完自动播放下一集")
                onToggled: function () {
                    if (playback.backend)
                        playback.backend.setAutoPlayNext(checked)
                }
            }
        }

        KSettingsRow {
            objectName: "settingsAdvancedPlayer"
            visible: !playback.flatpak
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
            visible: playback.advancedOpen && !playback.flatpak
            Layout.fillWidth: true
            Layout.topMargin: theme.gapMd
            spacing: 0

            KSettingsRow {
                title: qsTr("播放器命令")
                description: qsTr("留空使用当前播放器的默认命令。")
                controlWidth: 360

                KTextField {
                    id: commandField
                    objectName: "settingsPlayerCommand"
                    width: parent.width
                    selectByMouse: true
                    placeholderText: playback.backend ? playback.backend.playerDefaultCommand : ""
                    text: playback.backend ? playback.backend.playerCommand : ""
                    onEditingFinished: playback.applyPlayer()
                }
            }

            KSettingsRow {
                title: qsTr("附加参数")
                description: qsTr("按需追加播放参数；输入框可留空使用播放器默认值。")
                controlWidth: 360

                KTextField {
                    id: argsField
                    objectName: "settingsPlayerArgs"
                    width: parent.width
                    selectByMouse: true
                    placeholderText: playback.backend ? playback.backend.playerDefaultArgs : ""
                    text: playback.backend ? playback.backend.playerArgs : ""
                    onEditingFinished: playback.applyPlayer()
                }
            }

            KSettingsRow {
                title: qsTr("占位符")
                controlWidth: 360

                Label {
                    objectName: "settingsPlayerArgsHint"
                    width: parent.width
                    text: qsTr("非 mpv/VLC 播放器：占位符可放在命令或附加参数里；附加参数留空且命令里没有输入占位符时，默认附加参数是 {playlist}（直接传临时 M3U 路径）。需要 --playlist= 形式时自己写 --playlist={playlist}。{title} 是文件名，{url} 是签名直链；只使用 {url} 时不创建 M3U，且直链会出现在本机进程命令行中。原生构建可用 flatpak run --die-with-parent io.mpv.Mpv 调用 Flatpak 版 mpv。")
                    font.pixelSize: theme.fontCaption
                    color: theme.textMuted
                    wrapMode: Text.WordWrap
                }
            }

            KSettingsRow {
                title: qsTr("恢复默认")
                description: qsTr("清除已保存的自定义命令与附加参数。")

                KButton {
                    text: qsTr("恢复默认")
                    variant: "ghost"
                    onClicked: playback.resetPlayer()
                }
            }
        }
    }

    KSettingsGroup {
        title: qsTr("内嵌播放器")
        Layout.fillWidth: true

        SettingsPlaybackLanguage {
            Layout.fillWidth: true
            backend: playback.backend
        }

        KSettingsRow {
            objectName: "settingsVulkan"
            title: qsTr("Vulkan 渲染")
            description: qsTr("默认开启；播放异常时可关闭，改用 OpenGL，下次开播生效。")

            Switch {
                objectName: "settingsVulkanEnabled"
                checked: playback.backend ? playback.backend.playerVulkan : true
                Accessible.name: qsTr("使用 Vulkan 渲染")
                onToggled: function () {
                    if (playback.backend)
                        playback.backend.setPlayerVulkan(checked)
                }
            }
        }

        KSettingsRow {
            objectName: "settingsMpvConf"
            title: qsTr("embed-mpv.conf")
            description: playback.backend
                ? qsTr("读取画质等选项；内嵌必需的选项会被覆盖，下次开播生效。文件：%1").arg(playback.backend.embedMpvConfPath)
                : qsTr("读取画质等选项；内嵌必需的选项会被覆盖，下次开播生效。")

            RowLayout {
                spacing: theme.gapSm

                KButton {
                    objectName: "settingsMpvConfEdit"
                    text: qsTr("编辑")
                    variant: "ghost"
                    onClicked: {
                        if (playback.backend)
                            playback.backend.openPlayerMpvConf()
                    }
                }

                Switch {
                    objectName: "settingsMpvConfEnabled"
                    checked: playback.backend ? playback.backend.playerMpvConf : true
                    Accessible.name: qsTr("使用 embed-mpv.conf")
                    onToggled: function () {
                        if (playback.backend)
                            playback.backend.setPlayerMpvConf(checked)
                    }
                }
            }
        }

        KSettingsRow {
            objectName: "settingsInputConf"
            title: qsTr("embed-input.conf")
            description: playback.backend
                ? qsTr("读取键盘绑定；Esc / F / 方向键由 kichi 占用，下次开播生效。文件：%1").arg(playback.backend.embedInputConfPath)
                : qsTr("读取键盘绑定；Esc / F / 方向键由 kichi 占用，下次开播生效。")

            RowLayout {
                spacing: theme.gapSm

                KButton {
                    objectName: "settingsInputConfEdit"
                    text: qsTr("编辑")
                    variant: "ghost"
                    onClicked: {
                        if (playback.backend)
                            playback.backend.openPlayerInputConf()
                    }
                }

                Switch {
                    objectName: "settingsInputConfEnabled"
                    checked: playback.backend ? playback.backend.playerInputConf : true
                    Accessible.name: qsTr("使用 embed-input.conf")
                    onToggled: function () {
                        if (playback.backend)
                            playback.backend.setPlayerInputConf(checked)
                    }
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
    }

    // 后端值变化时同步到设置页控件。
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
