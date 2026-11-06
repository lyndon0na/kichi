import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

import io.kichi 1.0

// 内嵌播放窗口：libmpv 画面 + 最简控制条，独立于主窗口。
//
// 只在 Rust 侧 Ready（画面已出）后由 embedActive 拉起；播放状态仍归 backend，
// 这里只按固定节拍读位置 / 时长，并把暂停、拖动、关闭发回去。
// backend 所有属性共用一个 changed 信号，位置这类高频值不做绑定，避免每次
// 通知都重算窗口绑定。
//
// 关窗语义（docs/pitfalls.md 第 60 条）：
// - 用户关窗（窗口管理器 / Alt+F4 / 关闭按钮）= 停播；`dismissed` 压住
//   「Stopped 回投之前又被 backend.changed 拉起来」的窗口。
// - 播放结束 / 失败由 backend 把 embedActive 置 false，这里只 `hide()`：
//   走 hide 不会触发 onClosing，也就不会误把刚起步的新一次播放停掉。
Window {
    id: win

    property var backend: null
    readonly property bool playing: backend !== null && backend.embedActive === true
    readonly property bool isFullScreen: win.visibility === Window.FullScreen

    // 用户已经关窗，但 backend 还没把 embedActive 置 false：期间不要自动重开。
    property bool dismissed: false
    // 隐藏时 visibility 只剩 Hidden，用它记住「收起前是不是全屏」，
    // 下一次开播仍按初始尺寸在窗口态打开。
    property bool wasFullScreen: false

    property real position: 0
    property real duration: 0
    property bool paused: false
    property bool seeking: false
    property bool chromeVisible: true
    // 音量 / 静音由 backend 持久化；这里只绑定展示。
    readonly property int volume: backend !== null ? backend.embedVolume : 100
    readonly property bool muted: backend !== null ? backend.embedMuted : false
    // 调音量 / 静音的瞬时提示（不依赖控制条是否可见）
    property string osdText: ""
    // 点击喇叭打开的音量面板（滑块 + 静音）
    property bool volumeOpen: false
    // 拖动松手后等 mpv 回报新位置，期间别让旧坐标把滑块拉回去。
    property real seekTarget: -1

    title: backend !== null && backend.embedTitle.length > 0
           ? backend.embedTitle : "kichi"
    color: "black"
    // 初始 1280×720，按屏幕可用区域等比限幅；最小 480×270。
    width: 1280
    height: 720
    minimumWidth: 480
    minimumHeight: 270
    visible: false

    function applyInitialGeometry() {
        var scr = win.screen
        var screenW = scr && scr.width > 0 ? scr.width : 1280
        var screenH = scr && scr.height > 0 ? scr.height : 720
        // 可用区域扣掉了面板 / 任务栏；没有该属性时退回整屏尺寸。
        var availW = scr && scr.desktopAvailableWidth > 0 ? scr.desktopAvailableWidth : screenW
        var availH = scr && scr.desktopAvailableHeight > 0 ? scr.desktopAvailableHeight : screenH
        var scale = Math.min(1, availW / 1280, availH / 720)
        win.width = Math.max(480, Math.round(1280 * scale))
        win.height = Math.max(270, Math.round(720 * scale))
        if (scr) {
            win.x = Math.round(scr.virtualX + (screenW - win.width) / 2)
            win.y = Math.round(scr.virtualY + (screenH - win.height) / 2)
        }
    }

    function openPlayback() {
        // 上一次是全屏的话先退回窗口态，再按初始尺寸居中打开。
        if (wasFullScreen || win.visibility === Window.FullScreen)
            win.showNormal()
        wasFullScreen = false
        applyInitialGeometry()
        dismissed = false
        resetPlayback()
        win.show()
        win.raise()
        win.requestActivate()
        Qt.callLater(function () {
            if (win.visible)
                surface.forceActiveFocus()
        })
        syncPlayback()
        wake()
    }

    function closePlayback() {
        if (win.visible)
            win.hide()
    }

    // 用户主动关闭：立刻停播并收起，不等 Stopped 回投。
    function requestClose() {
        if (!win.visible)
            return
        dismissed = true
        commitAudio()
        if (backend !== null)
            backend.embedStop()
        closePlayback()
    }

    // backend 的状态变化（共用 changed 信号）统一在这里对账。
    function syncVisibility() {
        if (!win.playing) {
            if (win.visible)
                closePlayback()
            commitAudio()
            dismissed = false
            resetPlayback()
            return
        }
        if (!win.visible && !win.dismissed)
            openPlayback()
    }

    function syncPlayback() {
        if (!win.visible || backend === null)
            return
        duration = Math.max(0, backend.embedDuration())
        paused = backend.embedPaused()
        if (!seeking && seekTarget < 0) {
            position = Math.max(0, backend.embedPosition())
            seekSlider.value = position
        }
        // 面板开着时，键盘 / 滚轮改的音量也要同步到滑块（拖动中不抢）。
        if (!volumeSlider.pressed)
            volumeSlider.value = win.volume
    }

    function togglePause() {
        if (!win.playing || backend === null)
            return
        backend.embedTogglePause()
        syncPlayback()
        wake()
    }

    function seek(target) {
        if (!win.playing || backend === null || duration <= 0)
            return
        target = Math.max(0, Math.min(duration, target))
        position = target
        seekSlider.value = target
        seekTarget = target
        seekSettle.restart()
        backend.embedSeek(target)
        wake()
    }

    function seekBy(delta) {
        seek(position + delta)
    }

    function beginSeek() {
        seeking = true
        seekSettle.stop()
        hideTimer.stop()
    }

    function endSeek(target) {
        seeking = false
        seekTarget = target
        backend.embedSeek(target)
        position = target
        seekSlider.value = target
        seekSettle.restart()
        wake()
    }

    function volumeIconName() {
        if (win.muted || win.volume <= 0)
            return "volume-x"
        if (win.volume < 34)
            return "volume"
        if (win.volume < 67)
            return "volume-2"
        return "volume-3"
    }

    function setVolume(value) {
        if (!win.playing || backend === null)
            return
        value = Math.max(0, Math.min(100, Math.round(value)))
        // 静音时调音量顺手取消静音，否则按方向键看起来毫无反应。
        if (win.muted)
            backend.embedToggleMute()
        backend.embedSetVolume(value)
        audioSave.restart()
        // 面板本来就显示百分比，别再叠一层 OSD。
        if (!win.volumeOpen)
            flashOsd(qsTr("音量 %1%").arg(value))
    }

    function adjustVolume(delta) {
        setVolume(win.volume + delta)
    }

    function toggleMute() {
        if (!win.playing || backend === null)
            return
        var muted = !win.muted
        backend.embedToggleMute()
        audioSave.restart()
        if (!win.volumeOpen)
            flashOsd(muted ? qsTr("静音") : qsTr("音量 %1%").arg(win.volume))
    }

    // 音量 / 静音在停止调节 0.5 秒后统一写盘，拖动滑块不会刷爆设置文件。
    function commitAudio() {
        audioSave.stop()
        if (backend !== null)
            backend.embedCommitAudio()
    }

    function toggleVolumePanel() {
        if (!win.playing)
            return
        volumeOpen = !volumeOpen
        if (volumeOpen)
            volumeSlider.value = win.volume
        wake()
    }

    function flashOsd(text) {
        osdText = text
        osdTimer.restart()
    }

    function toggleFullscreen() {
        if (win.isFullScreen)
            win.showNormal()
        else
            win.showFullScreen()
        wake()
    }

    function resetPlayback() {
        hideTimer.stop()
        seekSettle.stop()
        osdTimer.stop()
        osdText = ""
        volumeOpen = false
        position = 0
        duration = 0
        paused = false
        seeking = false
        seekTarget = -1
        chromeVisible = true
    }

    function formatTime(seconds) {
        var total = Math.max(0, Math.floor(seconds))
        var h = Math.floor(total / 3600)
        var m = Math.floor((total % 3600) / 60)
        var s = total % 60
        var mm = (m < 10 ? "0" : "") + m
        var ss = (s < 10 ? "0" : "") + s
        return h > 0 ? h + ":" + mm + ":" + ss : mm + ":" + ss
    }

    function wake() {
        if (!win.visible)
            return
        chromeVisible = true
        // 暂停 / 拖动时控制条一直留着；播放中才按闲置时间收起。
        if (paused || seeking || volumeOpen)
            hideTimer.stop()
        else
            hideTimer.restart()
    }

    onPlayingChanged: syncVisibility()
    onBackendChanged: syncVisibility()
    Component.onCompleted: syncVisibility()

    // 隐藏时 visibility 变成 Hidden；只记住进入 / 离开全屏的那两次变化。
    onVisibilityChanged: function (visibility) {
        if (visibility === Window.FullScreen)
            wasFullScreen = true
        else if (visibility === Window.Windowed)
            wasFullScreen = false
    }

    onClosing: function (close) {
        // 用户关窗 = 停播；Stopped 回投前保持 dismissed，别被 changed 拉回来。
        win.dismissed = true
        win.commitAudio()
        if (win.backend !== null)
            win.backend.embedStop()
    }

    onPausedChanged: {
        if (paused) {
            chromeVisible = true
            hideTimer.stop()
        } else if (win.visible) {
            wake()
        }
    }

    Connections {
        target: win.backend
        function onChanged() {
            if (win.visible)
                win.syncPlayback()
        }
    }

    Timer {
        id: syncTimer
        interval: 250
        repeat: true
        running: win.visible && win.playing
        onTriggered: win.syncPlayback()
    }

    // 播放中闲置后收起控制条；暂停、拖动、鼠标停在控制条上时一直可见。
    Timer {
        id: hideTimer
        interval: 2500
        onTriggered: {
            if (win.paused || win.seeking || win.volumeOpen
                    || topHover.hovered || bottomBarHover.hovered) {
                restart()
                return
            }
            win.chromeVisible = false
        }
    }

    Timer {
        id: seekSettle
        interval: 900
        onTriggered: {
            win.seekTarget = -1
            win.syncPlayback()
        }
    }

    Timer {
        id: osdTimer
        interval: 900
        onTriggered: win.osdText = ""
    }

    Timer {
        id: audioSave
        interval: 500
        onTriggered: win.commitAudio()
    }

    Rectangle {
        id: surface

        anchors.fill: parent
        color: "black"
        focus: true

        KTheme { id: theme }

        MpvVideo {
            id: video
            anchors.fill: parent
            active: win.visible && win.playing
        }

        Label {
            anchors.centerIn: parent
            width: Math.min(parent.width - 64, 560)
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.WordWrap
            visible: video.error.length > 0
            text: video.error
            textFormat: Text.PlainText
            color: "white"
            font.pixelSize: theme.fontBody
        }

        // 点画面切换控制条；双击切换全屏。控制条是本项之后的兄弟，先拿到鼠标事件。
        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.LeftButton
            hoverEnabled: true
            onClicked: {
                // 音量面板开着时，点画面先收起面板，不顺手把控制条也藏掉。
                if (win.volumeOpen) {
                    win.volumeOpen = false
                    win.wake()
                } else if (win.chromeVisible) {
                    hideTimer.stop()
                    win.chromeVisible = false
                } else {
                    win.wake()
                }
            }
            onDoubleClicked: win.toggleFullscreen()
            // 鼠标在窗口里任何一次移动都唤醒控制条（收起状态下也一样）。
            onEntered: win.wake()
            onPositionChanged: win.wake()
        }

        // 滚轮调音量，与常见播放器一致；上下滚动一步 5%。
        WheelHandler {
            onWheel: function (event) {
                if (event.angleDelta.y === 0)
                    return
                win.adjustVolume(event.angleDelta.y > 0 ? 5 : -5)
                event.accepted = true
            }
        }

        Rectangle {
            id: topBar
            anchors.top: parent.top
            anchors.left: parent.left
            anchors.right: parent.right
            height: 64
            enabled: win.chromeVisible
            opacity: win.chromeVisible ? 1 : 0
            Behavior on opacity { NumberAnimation { duration: theme.motionFast } }
            gradient: Gradient {
                GradientStop { position: 0.0; color: "#B3000000" }
                GradientStop { position: 1.0; color: "transparent" }
            }

            // 鼠标停在控制条上时保持可见；点空白处不要穿透去切换画面。
            HoverHandler {
                id: topHover
                onHoveredChanged: if (hovered) win.wake()
            }

            MouseArea {
                anchors.fill: parent
                acceptedButtons: Qt.LeftButton
                hoverEnabled: true
                onEntered: win.wake()
                onPositionChanged: win.wake()
                onClicked: win.wake()
            }

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: theme.gapLg
                anchors.rightMargin: theme.gapMd
                anchors.topMargin: theme.gapSm
                spacing: theme.gapSm

                ElidedLabel {
                    Layout.fillWidth: true
                    text: win.backend ? win.backend.embedTitle : ""
                    elide: Text.ElideRight
                    textFormat: Text.PlainText
                    color: "white"
                    font.pixelSize: theme.fontSection
                    font.weight: Font.Medium
                }

                // 窗口态由窗管标题栏提供关闭；全屏时没有标题栏，才在控制条上留出口。
                KIconButton {
                    objectName: "playbackWindowClose"
                    visible: win.isFullScreen
                    iconName: "x"
                    iconSize: 18
                    iconColor: "white"
                    iconOpacity: 1
                    Accessible.name: qsTr("关闭播放")
                    ToolTip.visible: hovered
                    ToolTip.text: qsTr("关闭播放")
                    onClicked: win.requestClose()
                }
            }
        }

        Rectangle {
            id: bottomBar
            anchors.bottom: parent.bottom
            anchors.left: parent.left
            anchors.right: parent.right
            height: 84
            enabled: win.chromeVisible
            opacity: win.chromeVisible ? 1 : 0
            Behavior on opacity { NumberAnimation { duration: theme.motionFast } }
            gradient: Gradient {
                GradientStop { position: 0.0; color: "transparent" }
                GradientStop { position: 1.0; color: "#B3000000" }
            }

            // 鼠标停在控制条上时保持可见；点空白处不要穿透去切换画面。
            HoverHandler {
                id: bottomBarHover
                onHoveredChanged: if (hovered) win.wake()
            }

            MouseArea {
                anchors.fill: parent
                acceptedButtons: Qt.LeftButton
                hoverEnabled: true
                onEntered: win.wake()
                onPositionChanged: win.wake()
                onClicked: win.wake()
            }

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: theme.gapLg
                anchors.rightMargin: theme.gapLg
                anchors.topMargin: theme.gapLg
                anchors.bottomMargin: theme.gapMd
                spacing: theme.gapMd

                KIconButton {
                    objectName: "playbackWindowPause"
                    iconName: win.paused ? "play" : "pause"
                    iconFilled: true
                    iconSize: 20
                    iconColor: "white"
                    iconOpacity: 1
                    implicitWidth: 34
                    implicitHeight: 34
                    Accessible.name: win.paused ? qsTr("继续播放") : qsTr("暂停")
                    ToolTip.visible: hovered
                    ToolTip.text: Accessible.name
                    onClicked: win.togglePause()
                }

                Label {
                    text: win.formatTime(win.position)
                    color: "white"
                    font.pixelSize: theme.fontMeta
                }

                Slider {
                    id: seekSlider
                    objectName: "playbackWindowSeek"
                    Layout.fillWidth: true
                    // 整条控制条都是拖动命中区，不只是那条 3px 的线。
                    Layout.fillHeight: true
                    implicitHeight: 32
                    // 方向键留给「±10 秒」快捷键，滑块不抢焦点。
                    focusPolicy: Qt.NoFocus
                    from: 0
                    to: Math.max(1, win.duration)
                    // value 由 syncPlayback / 拖动显式赋值：Slider 自己改 value
                    // 会悄悄断掉绑定，之后进度条就不再跟着播放走。
                    value: 0
                    enabled: win.duration > 0
                    onMoved: if (win.seeking) win.position = value
                    onPressedChanged: {
                        if (pressed) {
                            win.beginSeek()
                        } else if (win.seeking) {
                            win.endSeek(value)
                        }
                    }

                    background: Rectangle {
                        x: seekSlider.leftPadding
                        y: seekSlider.topPadding + seekSlider.availableHeight / 2 - height / 2
                        width: seekSlider.availableWidth
                        height: 3
                        radius: height / 2
                        color: "#55ffffff"

                        Rectangle {
                            width: seekSlider.visualPosition * parent.width
                            height: parent.height
                            radius: parent.radius
                            color: "white"
                        }
                    }

                    handle: Rectangle {
                        x: seekSlider.leftPadding
                           + seekSlider.visualPosition * (seekSlider.availableWidth - width)
                        y: seekSlider.topPadding + seekSlider.availableHeight / 2 - height / 2
                        width: 10
                        height: 10
                        radius: 5
                        color: "white"
                        visible: seekSlider.enabled
                                 && (seekSlider.pressed || seekSlider.hovered)
                    }
                }

                Label {
                    text: win.formatTime(win.duration)
                    color: "white"
                    font.pixelSize: theme.fontMeta
                }

                KIconButton {
                    objectName: "playbackWindowVolume"
                    iconName: win.volumeIconName()
                    iconSize: 18
                    iconColor: "white"
                    iconOpacity: 1
                    implicitWidth: 34
                    implicitHeight: 34
                    Accessible.name: qsTr("音量控制")
                    ToolTip.visible: hovered
                    ToolTip.text: win.muted
                                  ? qsTr("已静音（点击调节音量）")
                                  : qsTr("音量 %1%（点击调节，M 静音）").arg(win.volume)
                    onClicked: win.toggleVolumePanel()
                }

                KIconButton {
                    objectName: "playbackWindowFullscreen"
                    iconName: win.isFullScreen ? "minimize" : "maximize"
                    iconSize: 18
                    iconColor: "white"
                    iconOpacity: 1
                    implicitWidth: 34
                    implicitHeight: 34
                    Accessible.name: win.isFullScreen ? qsTr("退出全屏") : qsTr("全屏")
                    ToolTip.visible: hovered
                    ToolTip.text: Accessible.name
                    onClicked: win.toggleFullscreen()
                }
            }
        }

        // 点击喇叭弹出的音量面板：静音开关 + 滑块 + 百分比。
        Rectangle {
            id: volumePanel

            anchors.right: parent.right
            anchors.rightMargin: theme.gapLg
            anchors.bottom: bottomBar.top
            anchors.bottomMargin: theme.gapSm
            width: 212
            height: 44
            radius: theme.radiusControl
            color: "#CC000000"
            opacity: win.volumeOpen ? 1 : 0
            visible: opacity > 0
            Behavior on opacity { NumberAnimation { duration: theme.motionFast } }

            HoverHandler {
                onHoveredChanged: if (hovered) win.wake()
            }

            // 吃掉面板上的点击，别让它们穿透到画面去切换控制条。
            MouseArea {
                anchors.fill: parent
                acceptedButtons: Qt.LeftButton
                hoverEnabled: true
                onEntered: win.wake()
                onClicked: win.wake()
            }

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: theme.gapSm
                anchors.rightMargin: theme.gapMd
                spacing: theme.gapSm

                KIconButton {
                    objectName: "playbackWindowPanelMute"
                    iconName: win.muted ? "volume-x" : "volume-2"
                    iconSize: 16
                    iconColor: "white"
                    iconOpacity: 1
                    implicitWidth: 28
                    implicitHeight: 28
                    Accessible.name: win.muted ? qsTr("取消静音") : qsTr("静音")
                    ToolTip.visible: hovered
                    ToolTip.text: Accessible.name
                    onClicked: win.toggleMute()
                }

                Slider {
                    id: volumeSlider
                    objectName: "playbackWindowVolumeSlider"
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    implicitHeight: 28
                    // 方向键留给音量快捷键，滑块不抢焦点。
                    focusPolicy: Qt.NoFocus
                    from: 0
                    to: 100
                    stepSize: 5
                    value: 0
                    onMoved: win.setVolume(value)

                    background: Rectangle {
                        x: volumeSlider.leftPadding
                        y: volumeSlider.topPadding + volumeSlider.availableHeight / 2 - height / 2
                        width: volumeSlider.availableWidth
                        height: 3
                        radius: height / 2
                        color: "#55ffffff"

                        Rectangle {
                            width: volumeSlider.visualPosition * parent.width
                            height: parent.height
                            radius: parent.radius
                            color: "white"
                        }
                    }

                    handle: Rectangle {
                        x: volumeSlider.leftPadding
                           + volumeSlider.visualPosition * (volumeSlider.availableWidth - width)
                        y: volumeSlider.topPadding + volumeSlider.availableHeight / 2 - height / 2
                        width: 10
                        height: 10
                        radius: 5
                        color: "white"
                        visible: volumeSlider.pressed || volumeSlider.hovered
                                 || volumeSlider.value !== win.volume
                    }
                }

                Label {
                    Layout.preferredWidth: 30
                    horizontalAlignment: Text.AlignRight
                    text: win.volume + "%"
                    color: "white"
                    font.pixelSize: theme.fontCaption
                }
            }
        }

        // 音量 / 静音的瞬时反馈；控制条收起时也能看见。
        Rectangle {
            id: osd

            anchors.horizontalCenter: parent.horizontalCenter
            anchors.bottom: bottomBar.top
            anchors.bottomMargin: theme.gapMd
            width: osdLabel.implicitWidth + theme.gapXl
            height: 32
            radius: theme.radiusControl
            color: "#B3000000"
            opacity: win.osdText.length > 0 ? 1 : 0
            visible: opacity > 0
            Behavior on opacity { NumberAnimation { duration: theme.motionFast } }

            Label {
                id: osdLabel
                anchors.centerIn: parent
                text: win.osdText
                color: "white"
                font.pixelSize: theme.fontMeta
            }
        }

        // 全屏时 Esc 先退出全屏，否则关闭窗口；空格暂停 / 继续，左右 ±10 秒。
        Shortcut {
            sequence: "Escape"
            enabled: win.visible
            onActivated: {
                if (win.volumeOpen)
                    win.volumeOpen = false
                else if (win.isFullScreen)
                    win.showNormal()
                else
                    win.requestClose()
            }
        }
        Shortcut {
            sequence: "Space"
            enabled: win.visible
            onActivated: win.togglePause()
        }
        Shortcut {
            sequence: "Left"
            enabled: win.visible
            onActivated: win.seekBy(-10)
        }
        Shortcut {
            sequence: "Right"
            enabled: win.visible
            onActivated: win.seekBy(10)
        }
        // 音量：上下 ±5%，M 静音，滚轮同上。
        Shortcut {
            sequence: "Up"
            enabled: win.visible
            onActivated: win.adjustVolume(5)
        }
        Shortcut {
            sequence: "Down"
            enabled: win.visible
            onActivated: win.adjustVolume(-5)
        }
        Shortcut {
            sequence: "M"
            enabled: win.visible
            onActivated: win.toggleMute()
        }
        // 统计叠加层与外部 mpv 按 i 的效果一致（再按一次收起）。
        Shortcut {
            sequence: "I"
            enabled: win.visible
            onActivated: if (win.backend !== null) win.backend.embedToggleStats()
        }
    }
}
