import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtQuick.Shapes

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

    // 显式断开主窗口的 transient 关系：否则 Qt 会把嵌套 Window 自动设成主窗口的
    // 子窗口，KWin 永远把播放窗口压在 kichi 上面，主窗口就没法正常浏览。
    // 代价是任务栏 / Alt-Tab 多一个独立条目（docs/adr/0027-embedded-mpv.md）。
    transientParent: null

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
    // mpv 正在等缓存 / 跳转 / 起播前空闲；持续一小会儿才亮加载环（见 syncBuffering）
    property bool buffering: false
    property bool showBuffering: false
    readonly property bool queueAdvancing: backend !== null && backend.queueAdvancing === true
    property bool chromeVisible: true
    // 音量 / 静音由 backend 持久化；这里只绑定展示。
    readonly property int volume: backend !== null ? backend.embedVolume : 100
    readonly property bool muted: backend !== null ? backend.embedMuted : false
    // 调音量 / 静音的瞬时提示（不依赖控制条是否可见）
    property string osdText: ""
    // 点击喇叭打开的音量面板（滑块 + 静音）
    property bool volumeOpen: false
    // 倍速（百分比，跨视频记住）/ 字幕样式面板
    property int speedPercent: 100
    // 章节起点（秒），只在开始播放时读一次
    property var chapters: []
    // 鼠标命中的章节下标（-1 = 不在任何刻度附近）/ 鼠标在窗口内的横坐标
    property int hoveredChapter: -1
    property real chapterHoverX: 0
    // 进度条轨道中心的纵坐标：提示气泡贴在这条线上方，而不是控制条顶上
    property real chapterTipY: 0
    property bool speedOpen: false
    property bool subStyleOpen: false
    readonly property bool menuOpen: trackMenu.visible || queuePanel.visible
    // 任何浮层 / 菜单打开时，键盘归浮层，kichi 快捷键暂停，也不下发给 mpv。
    readonly property bool overlayOpen: volumeOpen || subStyleOpen || speedOpen || menuOpen
    // 拖动松手后等 mpv 回报新位置，期间别让旧坐标把滑块拉回去。
    property real seekTarget: -1
    // 同一个播放窗口切下一集时，backend.embedSerial 会 +1；据此刷新章节 / 进度。
    property int embedSerialSeen: -1

    title: backend !== null && backend.embedTitle.length > 0
           ? backend.embedTitle : "kichi"
    color: "black"
    // 视频窗口固定深色：系统浅色主题下也不要出现白色 ToolTip / 悬停底 / 系统菜单。
    palette.window: "#0A0A0A"
    palette.windowText: "#F2FFFFFF"
    palette.base: "#0A0A0A"
    palette.text: "#F2FFFFFF"
    palette.button: "#161616"
    palette.buttonText: "#F2FFFFFF"
    palette.placeholderText: "#99FFFFFF"
    palette.mid: "#33FFFFFF"
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
        // 倍速跨视频记住：先读会话里的值，避免首帧 syncPlayback 把同一值再写一次盘。
        if (backend !== null)
            speedPercent = Math.round(backend.embedSpeed() * 100)
        chapters = backend !== null ? backend.embedChapters() : []
        embedSerialSeen = backend !== null ? backend.embedSerial : -1
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
        commitSpeed()
        commitAudio()
        if (backend !== null) {
            // 片尾自动切集可能还在取下一集：关窗时一并取消，别让声音在后台继续。
            backend.cancelPlayLoading()
            backend.embedStop()
        }
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
        // 同一个窗口切下一集：标题由 embedTitle 绑定，这里刷新章节与进度。
        if (backend.embedSerial !== embedSerialSeen) {
            embedSerialSeen = backend.embedSerial
            refreshSession()
        }
        duration = Math.max(0, backend.embedDuration())
        paused = backend.embedPaused()
        if (!seeking && seekTarget < 0) {
            position = Math.max(0, backend.embedPosition())
            seekSlider.value = position
        }
        // 面板开着时，键盘 / 滚轮改的音量也要同步到滑块（拖动中不抢）。
        if (!volumeSlider.pressed)
            volumeSlider.value = win.volume
        // 倍速：mpv 原生键位（`[` / `]`）改过时同步标签，并写回设置。
        var reportedSpeed = Math.round(backend.embedSpeed() * 100)
        if (reportedSpeed !== win.speedPercent) {
            win.speedPercent = reportedSpeed
            backend.embedSetSpeed(reportedSpeed)
        }
        // mpv 键位（m / 9 / 0 等）改的是 mpv 自己的音量：拉回界面并落盘。
        var actualVolume = backend.embedCurrentVolume()
        var actualMuted = backend.embedCurrentMuted()
        if (actualVolume !== win.volume || actualMuted !== win.muted) {
            backend.embedSyncAudio(actualVolume, actualMuted)
            audioSave.restart()
        }
        syncBuffering()
    }

    // 缓冲提示：等缓存 / 跳转 / 起播前空闲时亮加载环。持续 300ms 才显示，
    // 避免快速跳转或瞬时缓存抖动让界面闪一下。
    function syncBuffering() {
        var buffering = backend !== null && backend.embedBuffering() === true
        win.buffering = buffering
        if (!buffering || win.seeking) {
            bufferingDelay.stop()
            win.showBuffering = false
            return
        }
        if (!bufferingDelay.running && !win.showBuffering)
            bufferingDelay.restart()
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
        syncBuffering()
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
        if (volumeOpen) {
            win.speedOpen = false
            volumeSlider.value = win.volume
        }
        wake()
    }

    function formatSpeed(percent) {
        return (Math.round(percent) / 100) + "×"
    }

    // 拖动中只改 mpv 与界面，不写设置；松手 / 收面板时再提交。
    function previewSpeedPercent(percent) {
        if (!win.playing || backend === null)
            return
        percent = Math.max(50, Math.min(300, Math.round(percent)))
        win.speedPercent = percent
        backend.embedPreviewSpeed(percent)
        wake()
    }

    function commitSpeedPercent(percent) {
        if (!win.playing || backend === null)
            return
        percent = Math.max(50, Math.min(300, Math.round(percent)))
        win.speedPercent = percent
        backend.embedSetSpeed(percent)
        wake()
    }

    function commitSpeed() {
        if (win.speedOpen)
            commitSpeedPercent(win.speedPercent)
    }

    function toggleSpeedPanel() {
        if (!win.playing)
            return
        if (win.speedOpen) {
            win.speedOpen = false
            wake()
            return
        }
        win.volumeOpen = false
        win.subStyleOpen = false
        closeMenus()
        win.speedOpen = true
        wake()
    }

    function closeMenus() {
        trackMenu.close()
        queuePanel.close()
    }

    function openTrackMenu(kind, anchor) {
        if (!win.playing || backend === null)
            return
        win.volumeOpen = false
        win.subStyleOpen = false
        win.speedOpen = false
        trackMenu.kind = kind
        trackMenu.openAt(anchor)
        wake()
    }

    function toggleQueuePanel(anchor) {
        if (!win.playing || backend === null)
            return
        if (queuePanel.visible) {
            queuePanel.close()
            surface.forceActiveFocus()
            wake()
            return
        }
        win.volumeOpen = false
        win.subStyleOpen = false
        win.speedOpen = false
        trackMenu.close()
        queuePanel.openAbove(anchor)
        wake()
    }

    function toggleSubtitleStyle() {
        if (!win.playing)
            return
        if (win.subStyleOpen) {
            win.subStyleOpen = false
            surface.forceActiveFocus()
            wake()
            return
        }
        win.volumeOpen = false
        win.speedOpen = false
        closeMenus()
        win.subStyleOpen = true
        wake()
    }

    function handleEscape() {
        if (win.menuOpen)
            return
        if (win.subStyleOpen) {
            win.subStyleOpen = false
            surface.forceActiveFocus()
            wake()
            return
        }
        if (win.volumeOpen) {
            win.volumeOpen = false
            wake()
            return
        }
        if (win.speedOpen) {
            win.speedOpen = false
            wake()
            return
        }
        if (win.isFullScreen)
            win.showNormal()
    }

    // Qt 按键 → mpv 键名。可打印字符直接交原文（Shift 产生的符号 / 大写
    // 已经在 text 里），特殊键映射成 mpv 的 LEFT / PGUP 这类名字。
    function mpvKeyName(event) {
        var ctrl = (event.modifiers & Qt.ControlModifier) !== 0
        var alt = (event.modifiers & Qt.AltModifier) !== 0
        var shift = (event.modifiers & Qt.ShiftModifier) !== 0
        var key = event.key
        var name = ""

        switch (key) {
        case Qt.Key_Space: name = "SPACE"; break
        case Qt.Key_Left: name = "LEFT"; break
        case Qt.Key_Right: name = "RIGHT"; break
        case Qt.Key_Up: name = "UP"; break
        case Qt.Key_Down: name = "DOWN"; break
        case Qt.Key_Return:
        case Qt.Key_Enter: name = "ENTER"; break
        case Qt.Key_Tab: name = "TAB"; break
        case Qt.Key_Backspace: name = "BS"; break
        case Qt.Key_Home: name = "HOME"; break
        case Qt.Key_End: name = "END"; break
        case Qt.Key_PageUp: name = "PGUP"; break
        case Qt.Key_PageDown: name = "PGDN"; break
        case Qt.Key_Insert: name = "INS"; break
        case Qt.Key_Delete: name = "DEL"; break
        default:
            if (!ctrl && !alt && event.text.length === 1
                    && event.text.charCodeAt(0) >= 32)
                return event.text
            if (key >= Qt.Key_A && key <= Qt.Key_Z)
                name = String.fromCharCode(97 + key - Qt.Key_A)
            else if (key >= Qt.Key_0 && key <= Qt.Key_9)
                name = String.fromCharCode(key)
            else if (key >= Qt.Key_F1 && key <= Qt.Key_F12)
                name = "F" + (key - Qt.Key_F1 + 1)
            else
                return ""
        }

        var prefix = ""
        if (ctrl)
            prefix += "Ctrl+"
        if (alt)
            prefix += "Alt+"
        if (shift)
            prefix += "Shift+"
        return prefix + name
    }

    // kichi 的 Shortcut（Esc / F / 方向键）先接；这里只处理没接住的键，
    // 原样交给 mpv 的 input 层。
    function handleKey(event) {
        if (!win.visible || win.overlayOpen)
            return false
        // Esc / 单按 F 永远归 kichi，mpv 也拿不到。
        if (event.key === Qt.Key_Escape)
            return false
        if (event.key === Qt.Key_F && event.modifiers === Qt.NoModifier)
            return false
        var name = mpvKeyName(event)
        if (name.length === 0)
            return false
        if (win.backend !== null)
            win.backend.embedKeypress(name)
        wake()
        return true
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

    // 同一个播放窗口里的下一集：只重置进度与章节，不动窗口 / 全屏 / 音量面板。
    function refreshSession() {
        position = 0
        duration = 0
        paused = false
        seeking = false
        seekTarget = -1
        seekSlider.value = 0
        hoveredChapter = -1
        chapters = backend !== null ? backend.embedChapters() : []
        if (backend !== null)
            speedPercent = Math.round(backend.embedSpeed() * 100)
    }

    function resetPlayback() {
        hideTimer.stop()
        seekSettle.stop()
        osdTimer.stop()
        bufferingDelay.stop()
        buffering = false
        showBuffering = false
        osdText = ""
        volumeOpen = false
        subStyleOpen = false
        speedOpen = false
        closeMenus()
        speedPercent = 100
        chapters = []
        hoveredChapter = -1
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

    // 鼠标横坐标（窗口坐标）离哪个章节刻度最近；6px 内才算命中。
    function updateChapterHover(x) {
        chapterHoverX = x
        chapterTipY = seekSlider.mapToItem(
                    surface, 0, seekSlider.topPadding + seekSlider.availableHeight / 2).y
        if (x < 0 || duration <= 0 || seeking || overlayOpen || chapters.length === 0) {
            hoveredChapter = -1
            return
        }
        var usable = seekSlider.availableWidth
        if (usable <= 0) {
            hoveredChapter = -1
            return
        }
        var left = seekSlider.mapToItem(surface, seekSlider.leftPadding, 0).x
        var nearest = -1
        var best = 6
        for (var i = 0; i < chapters.length; ++i) {
            var time = chapters[i].time
            if (time <= 0.5 || time >= duration)
                continue
            var distance = Math.abs(left + time / duration * usable - x)
            if (distance <= 6 && distance < best) {
                best = distance
                nearest = i
            }
        }
        hoveredChapter = nearest
    }

    function chapterLabel(chapter, index) {
        if (!chapter)
            return ""
        var title = chapter.title ? chapter.title.trim() : ""
        return title.length > 0 ? title : qsTr("第 %1 章").arg(index + 1)
    }

    function wake() {
        if (!win.visible)
            return
        chromeVisible = true
        // 暂停 / 拖动 / 浮层打开时控制条一直留着；播放中才按闲置时间收起。
        if (paused || seeking || overlayOpen)
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
        if (win.backend !== null) {
            win.backend.cancelPlayLoading()
            win.backend.embedStop()
        }
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
            if (win.paused || win.seeking || win.overlayOpen
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

    // 缓冲提示的防闪烁延迟：短暂停顿不弹，持续等缓存 / 跳转才亮加载环。
    Timer {
        id: bufferingDelay
        interval: 300
        onTriggered: {
            if (win.buffering && !win.seeking)
                win.showBuffering = true
        }
    }

    Rectangle {
        id: surface

        anchors.fill: parent
        color: "black"
        focus: true

        Keys.onPressed: function (event) {
            if (win.handleKey(event))
                event.accepted = true
        }

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
            // 控制条收起时连指针一起藏起来：视频画面上不需要光标。
            // 控制条隐藏时 topBar / bottomBar 已 enabled=false，不会抢走光标；
            // 一动鼠标 onPositionChanged → wake()，控制条和指针一起恢复。
            cursorShape: win.visible && !win.chromeVisible
                         ? Qt.BlankCursor : Qt.ArrowCursor
            onClicked: {
                // 浮层开着时，点画面先收起浮层，不顺手把控制条也藏掉。
                if (win.volumeOpen) {
                    win.volumeOpen = false
                    win.wake()
                } else if (win.subStyleOpen) {
                    win.subStyleOpen = false
                    win.wake()
                } else if (win.speedOpen) {
                    win.speedOpen = false
                    win.wake()
                } else if (win.chromeVisible) {
                    hideTimer.stop()
                    win.chromeVisible = false
                } else {
                    win.wake()
                }
                surface.forceActiveFocus()
            }
            onDoubleClicked: win.toggleFullscreen()
            // 鼠标在窗口里任何一次移动都唤醒控制条（收起状态下也一样）。
            onEntered: win.wake()
            onPositionChanged: win.wake()
        }

        // 滚轮调音量，与常见播放器一致；上下滚动一步 5%。
        WheelHandler {
            onWheel: function (event) {
                // 浮层 / 菜单开着时滚轮留给它们（例如样式面板滚动）。
                if (event.angleDelta.y === 0 || win.overlayOpen)
                    return
                win.adjustVolume(event.angleDelta.y > 0 ? 5 : -5)
                event.accepted = true
            }
        }

        // 队列切集：旧画面已经结束，新一集还在取链 / 等窗口时显示加载环。
        // 播放窗口里只作提示，不能点击取消：取消入口留在主窗口（文件列表 /
        // 底栏提示条），关窗同样会停止这次加载。
        VideoLoadingOverlay {
            objectName: "playbackWindowLoading"

            anchors.fill: parent
            cornerRadius: 0
            visible: win.playing && win.queueAdvancing
            cancellable: false
            label: win.backend !== null && win.backend.playLoadingName.length > 0
                   ? qsTr("正在加载「%1」…").arg(win.backend.playLoadingName)
                   : qsTr("正在加载…")
        }

        // 播放中的缓冲 / 跳转 / 起播等待：mpv 画面会停在旧帧，用加载环明确告诉
        // 用户还在工作。不能点击取消（取消入口在暂停按钮 / 关闭窗口）。
        VideoLoadingOverlay {
            objectName: "playbackWindowBuffering"

            anchors.fill: parent
            cornerRadius: 0
            visible: win.playing && win.showBuffering && !win.queueAdvancing
                     && video.error.length === 0
            cancellable: false
            label: qsTr("正在缓冲…")
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
                    id: closeButton
                    objectName: "playbackWindowClose"
                    visible: win.isFullScreen
                    iconName: "x"
                    iconSize: 18
                    iconColor: "white"
                    iconOpacity: 1
                    onMedia: true
                    focusPolicy: Qt.NoFocus
                    Accessible.name: qsTr("关闭播放")
                    KMediaToolTip {
                        visible: closeButton.hovered && !win.overlayOpen
                        text: qsTr("关闭播放")
                    }
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
                    id: previousButton
                    objectName: "playbackWindowPrevious"
                    iconName: "skip-back"
                    iconSize: 18
                    iconColor: "white"
                    iconOpacity: 0.9
                    onMedia: true
                    implicitWidth: 34
                    implicitHeight: 34
                    focusPolicy: Qt.NoFocus
                    enabled: win.backend !== null && win.backend.playQueueIndex > 0
                    Accessible.name: qsTr("上一集")
                    KMediaToolTip {
                        visible: previousButton.hovered && !win.overlayOpen
                        text: qsTr("上一集")
                    }
                    onClicked: {
                        if (win.backend !== null)
                            win.backend.playPrevious()
                        win.wake()
                    }
                }

                KIconButton {
                    id: pauseButton
                    objectName: "playbackWindowPause"
                    iconName: win.paused ? "play" : "pause"
                    iconFilled: true
                    iconSize: 20
                    iconColor: "white"
                    iconOpacity: 1
                    onMedia: true
                    implicitWidth: 34
                    implicitHeight: 34
                    focusPolicy: Qt.NoFocus
                    Accessible.name: win.paused ? qsTr("继续播放") : qsTr("暂停")
                    KMediaToolTip {
                        visible: pauseButton.hovered && !win.overlayOpen
                        text: win.paused ? qsTr("继续播放") : qsTr("暂停")
                    }
                    onClicked: win.togglePause()
                }

                KIconButton {
                    id: nextButton
                    objectName: "playbackWindowNext"
                    iconName: "skip-forward"
                    iconSize: 18
                    iconColor: "white"
                    iconOpacity: 0.9
                    onMedia: true
                    implicitWidth: 34
                    implicitHeight: 34
                    focusPolicy: Qt.NoFocus
                    enabled: win.backend !== null
                             && win.backend.playQueueIndex >= 0
                             && win.backend.playQueueIndex < win.backend.playQueue.length - 1
                    Accessible.name: qsTr("下一集")
                    KMediaToolTip {
                        visible: nextButton.hovered && !win.overlayOpen
                        text: qsTr("下一集")
                    }
                    onClicked: {
                        if (win.backend !== null)
                            win.backend.playNext()
                        win.wake()
                    }
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

                    // 被动 hover：只用来找最近的章节刻度，不影响点击和拖动。
                    HoverHandler {
                        onPointChanged: {
                            var local = surface.mapFromItem(null,
                                                            point.scenePosition.x,
                                                            point.scenePosition.y)
                            win.updateChapterHover(local.x)
                        }
                        onHoveredChanged: if (!hovered) win.updateChapterHover(-1)
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

                        // 章节标记：轨道上方的小三角，悬停时变大变亮。
                        // 只做视觉标记，不接鼠标；跳转仍由整条滑块负责。
                        Repeater {
                            model: win.chapters

                            delegate: Shape {
                                id: chapterMark
                                readonly property bool active: index === win.hoveredChapter
                                readonly property real fraction: modelData.time / Math.max(1, win.duration)
                                x: Math.round(fraction * parent.width - width / 2)
                                y: -height - 3
                                width: active ? 9 : 7
                                height: active ? 5 : 4
                                visible: modelData.time > 0.5 && modelData.time < win.duration

                                ShapePath {
                                    fillColor: chapterMark.active ? "white" : "#D9FFFFFF"
                                    strokeWidth: 0
                                    startX: 0
                                    startY: 0
                                    PathLine { x: chapterMark.width; y: 0 }
                                    PathLine { x: chapterMark.width / 2; y: chapterMark.height }
                                    PathLine { x: 0; y: 0 }
                                }
                            }
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

                // 倍速：文字按钮，非原速时加亮，一眼看出当前不是 1×。
                Rectangle {
                    id: speedButton
                    objectName: "playbackWindowSpeed"
                    implicitWidth: 44
                    implicitHeight: 30
                    radius: theme.radiusSmall
                    color: speedHover.containsMouse ? theme.mediaHover : "transparent"
                    Accessible.role: Accessible.Button
                    Accessible.name: qsTr("播放速度 %1").arg(win.formatSpeed(win.speedPercent))
                    KMediaToolTip {
                        visible: speedHover.containsMouse && !win.overlayOpen
                        text: qsTr("播放速度")
                    }

                    MouseArea {
                        id: speedHover
                        anchors.fill: parent
                        hoverEnabled: true
                        acceptedButtons: Qt.LeftButton
                        cursorShape: Qt.PointingHandCursor
                        onClicked: win.toggleSpeedPanel()
                    }

                    Label {
                        anchors.centerIn: parent
                        text: win.formatSpeed(win.speedPercent)
                        color: "white"
                        opacity: win.speedPercent === 100 ? 0.75 : 1
                        font.pixelSize: theme.fontMeta
                        font.weight: win.speedPercent === 100 ? Font.Normal : Font.Medium
                    }
                }

                KIconButton {
                    id: audioButton
                    objectName: "playbackWindowAudio"
                    iconName: "music"
                    iconSize: 18
                    iconColor: "white"
                    iconOpacity: 0.9
                    onMedia: true
                    implicitWidth: 34
                    implicitHeight: 34
                    focusPolicy: Qt.NoFocus
                    Accessible.name: qsTr("音轨")
                    KMediaToolTip {
                        visible: audioButton.hovered && !win.overlayOpen
                        text: qsTr("切换音轨")
                    }
                    onClicked: win.openTrackMenu("audio", audioButton)
                }

                KIconButton {
                    id: subtitleButton
                    objectName: "playbackWindowSubtitle"
                    iconName: "subtitle"
                    iconSize: 18
                    iconColor: "white"
                    iconOpacity: 0.9
                    onMedia: true
                    implicitWidth: 34
                    implicitHeight: 34
                    focusPolicy: Qt.NoFocus
                    Accessible.name: qsTr("字幕")
                    KMediaToolTip {
                        visible: subtitleButton.hovered && !win.overlayOpen
                        text: qsTr("切换字幕 / 字幕样式")
                    }
                    onClicked: win.openTrackMenu("sub", subtitleButton)
                }

                KIconButton {
                    id: queueButton
                    objectName: "playbackWindowQueue"
                    iconName: "playlist"
                    iconSize: 18
                    iconColor: "white"
                    iconOpacity: 0.9
                    onMedia: true
                    implicitWidth: 34
                    implicitHeight: 34
                    focusPolicy: Qt.NoFocus
                    enabled: win.backend !== null && win.backend.playQueue.length > 1
                    Accessible.name: qsTr("播放列表")
                    KMediaToolTip {
                        visible: queueButton.hovered && !win.overlayOpen
                        text: qsTr("播放列表")
                    }
                    onClicked: win.toggleQueuePanel(queueButton)
                }

                KIconButton {
                    id: volumeButton
                    objectName: "playbackWindowVolume"
                    iconName: win.volumeIconName()
                    iconSize: 18
                    iconColor: "white"
                    iconOpacity: 1
                    onMedia: true
                    implicitWidth: 34
                    implicitHeight: 34
                    focusPolicy: Qt.NoFocus
                    Accessible.name: qsTr("音量控制")
                    KMediaToolTip {
                        visible: volumeButton.hovered && !win.overlayOpen
                        text: win.muted
                              ? qsTr("已静音（点击调节音量）")
                              : qsTr("音量 %1%（点击调节，M 静音）").arg(win.volume)
                    }
                    onClicked: win.toggleVolumePanel()
                }

                KIconButton {
                    id: fullscreenButton
                    objectName: "playbackWindowFullscreen"
                    iconName: win.isFullScreen ? "minimize" : "maximize"
                    iconSize: 18
                    iconColor: "white"
                    iconOpacity: 1
                    onMedia: true
                    implicitWidth: 34
                    implicitHeight: 34
                    focusPolicy: Qt.NoFocus
                    Accessible.name: win.isFullScreen ? qsTr("退出全屏") : qsTr("全屏")
                    KMediaToolTip {
                        visible: fullscreenButton.hovered && !win.overlayOpen
                        text: win.isFullScreen ? qsTr("退出全屏") : qsTr("全屏")
                    }
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
            radius: theme.mediaRadius
            color: theme.mediaSurface
            border.width: 1
            border.color: theme.mediaBorder
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
                    onMedia: true
                    implicitWidth: 28
                    implicitHeight: 28
                    focusPolicy: Qt.NoFocus
                    Accessible.name: win.muted ? qsTr("取消静音") : qsTr("静音")
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

        // 音轨 / 字幕轨菜单（打开时现读轨道列表）。
        PlaybackTrackMenu {
            id: trackMenu

            backend: win.backend
            maxPopupHeight: Math.max(140, win.height - 180)

            onTrackSelected: function (kind, id, label) {
                if (win.backend !== null)
                    win.backend.embedSelectTrack(kind, id)
                if (kind === "audio")
                    win.flashOsd(qsTr("音轨：%1").arg(label))
                else if (id < 0)
                    win.flashOsd(qsTr("字幕已关闭"))
                else
                    win.flashOsd(qsTr("字幕：%1").arg(label))
                Qt.callLater(function () { surface.forceActiveFocus() })
                win.wake()
            }

            onStyleRequested: {
                win.closeMenus()
                win.subStyleOpen = true
                win.wake()
            }

            onClosed: surface.forceActiveFocus()
        }

        // 连播队列：当前文件夹的可播放条目，点一行直接跳过去。
        PlaybackQueuePanel {
            id: queuePanel

            backend: win.backend
            maxPopupHeight: Math.max(160, win.height - 180)

            onClosed: {
                surface.forceActiveFocus()
                win.wake()
            }
        }

        PlaybackSpeedPanel {
            backend: win.backend
            open: win.speedOpen
            current: win.speedPercent
            anchorItem: bottomBar

            onSpeedEdited: function (percent) { win.previewSpeedPercent(percent) }
            onSpeedCommitted: function (percent) { win.commitSpeedPercent(percent) }
        }

        PlaybackSubtitlePanel {
            id: subtitlePanel

            backend: win.backend
            open: win.subStyleOpen
            anchorItem: bottomBar

            onCloseRequested: {
                win.subStyleOpen = false
                surface.forceActiveFocus()
                win.wake()
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

        // 章节刻度悬停提示：跟随鼠标，只在命中刻度附近时出现。
        Rectangle {
            id: chapterTip

            readonly property var chapter: win.hoveredChapter >= 0
                                           && win.hoveredChapter < win.chapters.length
                                           ? win.chapters[win.hoveredChapter] : null
            readonly property bool shown: chapter !== null && !win.seeking
                                          && !win.overlayOpen && win.osdText.length === 0
            // 贴着进度条上方 12px（留出小三角的位置），跟着鼠标横移，不压住轨道。
            y: Math.round(win.chapterTipY - height - theme.gapMd)
            width: tipLabel.implicitWidth + theme.gapXl
            height: 32
            radius: theme.radiusControl
            color: "#B3000000"
            visible: shown
            x: Math.max(theme.gapSm, Math.min(parent.width - width - theme.gapSm,
                                              win.chapterHoverX - width / 2))

            Label {
                id: tipLabel

                anchors.centerIn: parent
                text: chapterTip.chapter
                      ? win.chapterLabel(chapterTip.chapter, win.hoveredChapter) + " · "
                        + win.formatTime(chapterTip.chapter.time)
                      : ""
                color: "white"
                font.pixelSize: theme.fontMeta
            }
        }

        // 键盘优先级：
        // - Esc 永远归 kichi：先收浮层，再退全屏；不关窗，菜单自己处理 Esc。
        // - 单按 F 永远归 kichi（切换窗口全屏），mpv 的 f 不生效。
        // - 方向键归 kichi：← → 前后 10 秒，↑ ↓ 音量 ±5%。
        // - 其余按键（Space / m / i / [ ] / 数字等）全部交给 mpv 内置默认键位。
        // - 任何菜单 / 浮层打开时：键盘归浮层，kichi 快捷键暂停，也不下发给 mpv。
        Shortcut {
            sequence: "Escape"
            enabled: win.visible && !win.menuOpen
            onActivated: win.handleEscape()
        }
        Shortcut {
            sequence: "F"
            enabled: win.visible && !win.overlayOpen
            onActivated: win.toggleFullscreen()
        }
        Shortcut {
            sequence: "Left"
            enabled: win.visible && !win.overlayOpen
            onActivated: win.seekBy(-10)
        }
        Shortcut {
            sequence: "Right"
            enabled: win.visible && !win.overlayOpen
            onActivated: win.seekBy(10)
        }
        // 音量：上下 ±5%，滚轮同上。
        Shortcut {
            sequence: "Up"
            enabled: win.visible && !win.overlayOpen
            onActivated: win.adjustVolume(5)
        }
        Shortcut {
            sequence: "Down"
            enabled: win.visible && !win.overlayOpen
            onActivated: win.adjustVolume(-5)
        }
    }
}
