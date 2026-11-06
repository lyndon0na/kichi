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
        if (backend !== null)
            backend.embedStop()
        closePlayback()
    }

    // backend 的状态变化（共用 changed 信号）统一在这里对账。
    function syncVisibility() {
        if (!win.playing) {
            if (win.visible)
                closePlayback()
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
        if (paused || seeking)
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
            if (win.paused || win.seeking
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
                if (win.chromeVisible) {
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

                KIconButton {
                    objectName: "playbackWindowFullscreen"
                    iconName: win.isFullScreen ? "minimize" : "maximize"
                    iconSize: 18
                    iconColor: "white"
                    iconOpacity: 1
                    Accessible.name: win.isFullScreen ? qsTr("退出全屏") : qsTr("全屏")
                    ToolTip.visible: hovered
                    ToolTip.text: Accessible.name
                    onClicked: win.toggleFullscreen()
                }

                KIconButton {
                    objectName: "playbackWindowClose"
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
            }
        }

        // 全屏时 Esc 先退出全屏，否则关闭窗口；空格暂停 / 继续，左右 ±10 秒。
        Shortcut {
            sequence: "Escape"
            enabled: win.visible
            onActivated: {
                if (win.isFullScreen)
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
    }
}
