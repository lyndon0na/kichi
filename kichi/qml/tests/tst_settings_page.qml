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
    height: 1200

    // 替代 Kichi 桥接对象：只提供设置页读写的属性 / 方法。
    QtObject {
        id: backend
        property string preferredQuality: "ask"
        property bool autoSubtitles: true
        property string playerDefaultCommand: "mpv"
        property string playerCommand: ""
        property string playerDefaultArgs: "--no-border"
        property string playerArgs: ""
        property string downloadDir: ""
        property bool notifyEnabled: true
        property bool closeToTray: false
        property string storageBackend: "KWallet"
        property string settingsPath: "/tmp/kichi-settings.toml"

        // SettingsPlayback 监听后端的 `changed`（设置从别处刷新时同步控件）
        signal changed()

        function setPreferredQuality(value) { preferredQuality = value }
        function setAutoSubtitles(value) { autoSubtitles = value }
        function setPlayer(command, args) {
            playerCommand = command
            playerArgs = args
        }
        function resetPlayer() {}
        function setDownloadDir(value) { downloadDir = value }
        function openDownloadDir() {}
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
            width: 1000, height: 1200, backend: backend
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
}
