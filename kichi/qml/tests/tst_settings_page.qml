import QtQuick
import QtTest
import ".."

// 设置页「账号」一节。
//
// 回归：退出登录原来在侧栏账号卡右侧（一个 26px 的图标按钮），
// 2026-10-29 用户要求搬进设置页——低频、不可逆的操作离日常点击远一点。
// 这里锁住两件事：按钮在设置页里；点它发一次 signOutRequested，
// 由 main.qml 接住弹确认框（确认框本身不在这个文件里）。
TestCase {
    name: "SettingsPage"
    when: windowShown
    // 足够高，让「账号」一节整个落在可视区内（否则 mouseClick 点不到）
    width: 1000
    height: 2200

    // 替代 Kichi 桥接对象：只提供设置页读写的属性 / 方法。
    QtObject {
        id: backend
        property string preferredQuality: "ask"
        property bool autoSubtitles: true
        property string playerBackend: "mpv"
        property string playerDefaultCommand: "mpv"
        property string playerCommand: ""
        property string playerDefaultArgs: "--no-border"
        property string playerArgs: ""
        property string downloadDir: ""
        property int downloadParallel: 3
        property int downloadConnections: 4
        property int downloadRateMib: 0
        property bool autoResumeDownloads: true
        property bool notifyEnabled: true
        property bool closeToTray: false
        property string storageBackend: "KWallet"
        property string settingsPath: "/tmp/kichi-settings.toml"
        property int chooseDownloadDirCalls: 0
        property int openDownloadDirCalls: 0

        // SettingsPlayback 监听后端的 `changed`（设置从别处刷新时同步控件）
        signal changed()

        function setPreferredQuality(value) { preferredQuality = value }
        function setAutoSubtitles(value) { autoSubtitles = value }
        function setPlayerBackend(value) { playerBackend = value }
        function setPlayer(command, args) {
            playerBackend = "custom"
            playerCommand = command
            playerArgs = args
        }
        function resetPlayer() {}
        function setDownloadDir(value) { downloadDir = value }
        function setDownloadParallel(value) { downloadParallel = value }
        function setDownloadConnections(value) { downloadConnections = value }
        function setDownloadRateMib(value) { downloadRateMib = value }
        function setAutoResumeDownloads(value) { autoResumeDownloads = value }
        function chooseDownloadDir() { chooseDownloadDirCalls += 1 }
        function openDownloadDir() { openDownloadDirCalls += 1 }
        function setNotifyEnabled(value) { notifyEnabled = value }
        function setCloseToTray(value) { closeToTray = value }
    }

    Component {
        id: pageComponent
        SettingsPage {}
    }

    property var page: null

    function init() {
        page = pageComponent.createObject(parent, {
            width: 1000, height: 2200, backend: backend
        })
        verify(page !== null, "SettingsPage 创建失败")
    }

    function cleanup() {
        if (page)
            page.destroy()
        page = null
    }

    function test_sign_out_button_is_in_settings() {
        var btn = findChild(page, "settingsSignOut")
        verify(btn !== null, "设置页有「退出登录」按钮")
        verify(btn.visible, "按钮可见")
        compare(btn.text, "退出登录")
    }

    function test_clicking_sign_out_emits_once() {
        var btn = findChild(page, "settingsSignOut")
        verify(btn !== null, "设置页有「退出登录」按钮")

        var fired = 0
        page.signOutRequested.connect(function () { fired += 1 })

        mouseClick(btn)
        wait(40)
        compare(fired, 1, "点一次只发一次退出登录请求")
    }

    function test_advanced_player_is_collapsed_until_clicked() {
        var row = findChild(page, "settingsAdvancedPlayer")
        var body = findChild(page, "settingsPlayerAdvancedBody")
        verify(row !== null, "有高级播放器设置行")
        verify(body !== null, "有高级播放器设置内容")
        verify(!body.visible, "高级播放器设置默认收起")

        mouseClick(row)
        wait(40)
        verify(body.visible, "点高级播放器设置后展开")
    }

    function test_player_picker_applies_vlc() {
        var box = findChild(page, "settingsPlayerBackend")
        verify(box !== null, "有播放器选择下拉框")
        compare(box.textAt(0), "mpv")
        compare(box.textAt(1), "VLC")

        box.currentIndex = 1
        box.activated(1)
        wait(40)
        compare(backend.playerBackend, "vlc", "选择 VLC 后应写回播放器预设")
    }

    function test_download_dir_commits_when_editing_finishes() {
        var field = findChild(page, "settingsDownloadDir")
        verify(field !== null, "有下载目录输入框")

        field.text = "/tmp/kichi-downloads"
        field.editingFinished()
        compare(backend.downloadDir, "/tmp/kichi-downloads")
    }

    function test_quality_ask_label_is_compact() {
        var box = findChild(page, "settingsQuality")
        verify(box !== null, "有默认清晰度下拉框")
        compare(box.textAt(0), "每次询问")
    }

    function test_quality_combo_opens_from_a_click() {
        var box = findChild(page, "settingsQuality")
        verify(box !== null, "有默认清晰度下拉框")
        verify(!box.popup.visible, "默认清晰度弹层初始收起")

        var pressed = 0
        box.pressedChanged.connect(function () {
            if (box.pressed)
                pressed += 1
        })
        mouseClick(box)
        wait(50)

        var center = box.mapToItem(null, box.width / 2, box.height / 2)
        compare(pressed, 1, "点击确实到达默认清晰度控件（center="
                + center + "，size=" + box.width + "×" + box.height
                + "，visible=" + box.visible + "，enabled=" + box.enabled
                + "，opacity=" + box.opacity + "）")
        verify(box.popup.visible, "点默认清晰度后弹层打开（down=" + box.down
               + "，visible=" + box.visible + "，enabled=" + box.enabled
               + "，popup=" + box.popup + "）")
        box.popup.close()
    }

    function test_settings_path_is_right_aligned() {
        var path = findChild(page, "settingsPath")
        verify(path !== null, "有设置文件路径")
        compare(path.horizontalAlignment, Text.AlignRight)
    }

    function test_directory_picker_uses_backend() {
        var choose = findChild(page, "settingsChooseDownloadDir")
        verify(choose !== null, "有选择目录按钮")

        mouseClick(choose)
        wait(40)

        compare(backend.chooseDownloadDirCalls, 1, "点一次只请求一次系统选择器")
    }

    function test_download_throughput_controls_are_wired() {
        var parallel = findChild(page, "settingsDownloadParallel")
        var connections = findChild(page, "settingsDownloadConnections")
        var rate = findChild(page, "settingsDownloadRate")
        var autoResume = findChild(page, "settingsAutoResumeDownloads")
        verify(parallel !== null)
        verify(connections !== null)
        verify(rate !== null)
        verify(autoResume !== null)

        parallel.value = 5
        parallel.valueModified()
        connections.value = 6
        connections.valueModified()
        rate.value = 12
        rate.valueModified()
        autoResume.checked = false
        autoResume.toggled()

        compare(backend.downloadParallel, 5)
        compare(backend.downloadConnections, 6)
        compare(backend.downloadRateMib, 12)
        compare(backend.autoResumeDownloads, false)
    }
}
