import QtQuick
import QtTest
import ".."

// 设置对话框内容：左侧大纲切换分组，右侧只保留当前分组。
// 退出登录仍在「账号」里：按钮只发一次 signOutRequested，
// 确认框与清令牌由 SignOutDialog / backend 负责。
TestCase {
    name: "SettingsPage"
    when: windowShown
    width: 900
    height: 640

    // 替代 Kichi 桥接对象：只提供设置页读写的属性 / 方法。
    QtObject {
        id: backend
        property string preferredQuality: "ask"
        property bool autoSubtitles: true
        property bool autoPlayNext: true
        property string playerAudioLang: "auto"
        property string playerSubLang: "auto"
        property bool playerMpvConf: true
        property string embedMpvConfPath: "/tmp/kichi/embed-mpv.conf"
        property bool playerInputConf: true
        property string embedInputConfPath: "/tmp/kichi/embed-input.conf"
        property bool playerVulkan: true
        property string playerBackend: "mpv"
        property string playerDefaultCommand: "mpv"
        property string playerCommand: ""
        property string playerDefaultArgs: "--no-border"
        property string playerArgs: ""
        property string downloadDir: ""
        property int downloadParallel: 3
        property int downloadConnections: 4
        property int downloadRateMib: 0
        property int uploadParallel: 2
        property int uploadRateMib: 0
        property bool autoResumeDownloads: true
        property bool autoResumeUploads: false
        property bool localIndexEnabled: false
        property bool notifyEnabled: true
        property bool closeToTray: false
        property string storageBackend: "系统钥匙串"
        property string quotaTransferCloud: "已用 1.0 GB / 40.0 TB"
        property string quotaTransferDownload: "已用 0 B / 4.0 TB"
        property string quotaTransferUpload: "已用 512.0 MB / 1.0 TB"
        property string quotaTransferReset: "2026/12/1 00:00 SGT"
        property real quotaTransferCloudRatio: 0.5
        property real quotaTransferDownloadRatio: 0.25
        property real quotaTransferUploadRatio: -1
        property bool quotaLoading: false
        property string settingsPath: "/tmp/kichi-settings.toml"
        property int chooseDownloadDirCalls: 0
        property int openDownloadDirCalls: 0
        property int openMpvConfCalls: 0
        property int openInputConfCalls: 0

        // SettingsPlayback / SettingsTransfers 监听后端的 `changed`
        signal changed()

        function setPreferredQuality(value) { preferredQuality = value }
        function setAutoSubtitles(value) { autoSubtitles = value }
        function setAutoPlayNext(value) { autoPlayNext = value }
        function setPlayerAudioLang(value) { playerAudioLang = value }
        function setPlayerSubLang(value) { playerSubLang = value }
        function setPlayerMpvConf(value) { playerMpvConf = value }
        function openPlayerMpvConf() { openMpvConfCalls += 1 }
        function setPlayerInputConf(value) { playerInputConf = value }
        function openPlayerInputConf() { openInputConfCalls += 1 }
        function setPlayerVulkan(value) { playerVulkan = value }
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
        function setUploadParallel(value) { uploadParallel = value }
        function setUploadRateMib(value) { uploadRateMib = value }
        function setAutoResumeDownloads(value) { autoResumeDownloads = value }
        function setAutoResumeUploads(value) { autoResumeUploads = value }
        function setLocalIndexEnabled(value) { localIndexEnabled = value }
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
        // 每个用例都从默认值开始：后面的用例会改写这些字段
        backend.preferredQuality = "ask"
        backend.autoSubtitles = true
        backend.autoPlayNext = true
        backend.playerAudioLang = "auto"
        backend.playerSubLang = "auto"
        backend.playerMpvConf = true
        backend.playerInputConf = true
        backend.playerVulkan = true
        backend.playerBackend = "mpv"
        backend.playerCommand = ""
        backend.playerArgs = ""
        backend.downloadDir = ""
        backend.downloadParallel = 3
        backend.downloadConnections = 4
        backend.downloadRateMib = 0
        backend.uploadParallel = 2
        backend.uploadRateMib = 0
        backend.autoResumeDownloads = true
        backend.autoResumeUploads = false
        backend.localIndexEnabled = false
        backend.quotaTransferCloud = "已用 1.0 GB / 40.0 TB"
        backend.quotaTransferDownload = "已用 0 B / 4.0 TB"
        backend.quotaTransferUpload = "已用 512.0 MB / 1.0 TB"
        backend.quotaTransferReset = "2026/12/1 00:00 SGT"
        backend.quotaTransferCloudRatio = 0.5
        backend.quotaTransferDownloadRatio = 0.25
        backend.quotaTransferUploadRatio = -1
        backend.quotaLoading = false
        backend.notifyEnabled = true
        backend.closeToTray = false
        backend.chooseDownloadDirCalls = 0
        backend.openDownloadDirCalls = 0
        backend.openMpvConfCalls = 0
        backend.openInputConfCalls = 0

        page = pageComponent.createObject(parent, {
            width: 900, height: 640, backend: backend, visible: true
        })
        verify(page !== null, "SettingsPage 创建失败")
    }

    function cleanup() {
        if (page)
            page.destroy()
        page = null
    }

    function openSection(index) {
        page.currentSection = index
        wait(50)
    }

    function test_default_section_is_account() {
        compare(page.currentSection, page.accountSection, "默认停在「账号」")

        var cloud = findChild(page, "settingsTransferCloud")
        verify(cloud !== null, "账号页有云下载额度")
        verify(cloud.visible, "云下载额度可见")
    }

    function test_transfer_quota_shows_the_three_allowances() {
        openSection(page.accountSection)

        var cloud = findChild(page, "settingsTransferCloud")
        var download = findChild(page, "settingsTransferDownload")
        var upload = findChild(page, "settingsTransferUpload")
        var reset = findChild(page, "settingsTransferReset")
        verify(cloud !== null && download !== null && upload !== null && reset !== null)

        compare(cloud.text, backend.quotaTransferCloud)
        compare(download.text, backend.quotaTransferDownload)
        compare(upload.text, backend.quotaTransferUpload)
        compare(reset.text, backend.quotaTransferReset)
    }

    function test_transfer_quota_falls_back_while_loading() {
        openSection(page.accountSection)

        backend.quotaTransferCloud = ""
        backend.quotaLoading = true
        wait(20)
        var cloud = findChild(page, "settingsTransferCloud")
        compare(cloud.text, "获取中…")

        backend.quotaLoading = false
        wait(20)
        compare(cloud.text, "未知")
    }

    function test_transfer_quota_bars_follow_ratios() {
        openSection(page.accountSection)

        var cloud = findChild(page, "settingsTransferCloudBar")
        var download = findChild(page, "settingsTransferDownloadBar")
        var upload = findChild(page, "settingsTransferUploadBar")
        verify(cloud !== null && download !== null && upload !== null)

        compare(cloud.opacity, 1, "有上限的额度画进度条")
        compare(cloud.value, backend.quotaTransferCloudRatio)
        compare(download.value, backend.quotaTransferDownloadRatio)
        compare(upload.opacity, 0, "没有上限时不画条")
    }

    function test_section_switch_fades_content_in() {
        openSection(page.transfersSection)

        var loader = findChild(page, "settingsBodyLoader")
        verify(loader !== null, "有分组内容装载器")

        page.currentSection = page.accountSection
        if (page.theme.motionNormal > 0) {
            verify(loader.opacity < 1, "切换瞬间新内容从透明开始")
            verify(loader.y > 0, "新内容从下方轻微上移")
        }
        tryCompare(loader, "opacity", 1, 2000)
        compare(loader.y, 0)
    }

    function test_outline_selection_pill_slides_to_new_section() {
        openSection(page.accountSection)

        var pill = findChild(page, "settingsSelectionPill")
        var first = findChild(page, "settingsOutlineItem_" + page.accountSection)
        var second = findChild(page, "settingsOutlineItem_" + page.playbackSection)
        verify(pill !== null && first !== null && second !== null)
        verify(pill.visible, "选中药丸可见")

        const firstY = first.mapToItem(pill.parent, 0, 0).y
        const secondY = second.mapToItem(pill.parent, 0, 0).y
        compare(Math.round(pill.y), Math.round(firstY), "初始停在「账号」")
        compare(pill.height, first.height)

        page.currentSection = page.playbackSection
        if (page.theme.motionNormal > 0)
            verify(pill.y < secondY, "药丸从旧行滑向新行，不瞬移")

        tryCompare(pill, "y", secondY)
        compare(pill.height, second.height)
    }

    function test_outline_switches_section() {
        var transfersItem = findChild(page, "settingsOutlineItem_" + page.transfersSection)
        verify(transfersItem !== null, "大纲有「传输」入口")

        mouseClick(transfersItem)
        wait(50)

        compare(page.currentSection, page.transfersSection, "点大纲切到传输")
        verify(findChild(page, "settingsDownloadDir") !== null, "传输页出现下载目录")
        compare(findChild(page, "settingsPlayerBackend"), null, "上一组内容已卸掉")
    }

    function test_sign_out_button_is_in_account_section() {
        openSection(page.accountSection)

        var btn = findChild(page, "settingsSignOut")
        verify(btn !== null, "账号页有「退出登录」按钮")
        verify(btn.visible, "按钮可见")
        compare(btn.text, "退出登录")
    }

    function test_clicking_sign_out_emits_once() {
        openSection(page.accountSection)

        var btn = findChild(page, "settingsSignOut")
        verify(btn !== null, "账号页有「退出登录」按钮")

        var fired = 0
        page.signOutRequested.connect(function () { fired += 1 })

        mouseClick(btn)
        wait(40)
        compare(fired, 1, "点一次只发一次退出登录请求")
    }

    function test_advanced_player_is_collapsed_until_clicked() {
        openSection(page.playbackSection)

        var row = findChild(page, "settingsAdvancedPlayer")
        var body = findChild(page, "settingsPlayerAdvancedBody")
        verify(row !== null, "有高级播放器设置行")
        verify(body !== null, "有高级播放器设置内容")
        verify(!body.visible, "高级播放器设置默认收起")

        mouseClick(row)
        wait(40)
        verify(body.visible, "点高级播放器设置后展开")
    }

    function test_custom_player_placeholder_hint_is_visible() {
        openSection(page.playbackSection)

        var row = findChild(page, "settingsAdvancedPlayer")
        var hint = findChild(page, "settingsPlayerArgsHint")
        verify(row !== null, "有高级播放器设置行")
        verify(hint !== null, "有自定义播放器参数提示")

        mouseClick(row)
        wait(40)

        verify(hint.visible, "展开后提示可见")
        verify(hint.text.indexOf("{title}") >= 0, "提示文件名占位符")
        verify(hint.text.indexOf("{playlist}") >= 0, "提示清单占位符")
        verify(hint.text.indexOf("{url}") >= 0, "提示直链占位符")
        verify(hint.text.indexOf("默认附加参数是 {playlist}") >= 0, "提示通用播放器默认值")
    }

    function test_player_picker_applies_vlc() {
        openSection(page.playbackSection)

        var box = findChild(page, "settingsPlayerBackend")
        verify(box !== null, "有播放器选择下拉框")
        compare(box.textAt(0), "内嵌（默认）")
        compare(box.textAt(1), "mpv")
        compare(box.textAt(2), "VLC")

        box.currentIndex = 2
        box.activated(2)
        wait(40)
        compare(backend.playerBackend, "vlc", "选择 VLC 后应写回播放器预设")
    }

    function test_quality_ask_label_is_compact() {
        openSection(page.playbackSection)

        var box = findChild(page, "settingsQuality")
        verify(box !== null, "有默认清晰度下拉框")
        compare(box.textAt(0), "每次询问")
    }

    function test_quality_combo_opens_from_a_click() {
        openSection(page.playbackSection)

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

        compare(pressed, 1, "点击确实到达默认清晰度控件")
        verify(box.popup.visible, "点默认清晰度后弹层打开")
        box.popup.close()
    }

    function test_language_preference_combos_are_wired() {
        openSection(page.playbackSection)

        var audio = findChild(page, "settingsAudioLanguage")
        var subtitle = findChild(page, "settingsSubtitleLanguage")
        verify(audio !== null, "有首选音轨语言下拉框")
        verify(subtitle !== null, "有首选字幕语言下拉框")
        compare(audio.textAt(0), "自动（跟随系统语言）")
        compare(subtitle.textAt(0), "自动（跟随系统语言）")

        audio.currentIndex = 4
        audio.activated(4)
        wait(40)
        compare(backend.playerAudioLang, "ja", "选择日语后写回首选音轨语言")

        subtitle.currentIndex = 2
        subtitle.activated(2)
        wait(40)
        compare(backend.playerSubLang, "yue", "选择粤语后写回首选字幕语言")

        backend.playerAudioLang = "ko"
        backend.playerSubLang = "en"
        backend.changed()
        wait(40)
        compare(audio.currentValue, "ko", "后端变化时音轨下拉跟随")
        compare(subtitle.currentValue, "en", "后端变化时字幕下拉跟随")
    }

    function test_embed_mpv_conf_is_wired() {
        openSection(page.playbackSection)

        var toggle = findChild(page, "settingsMpvConfEnabled")
        var edit = findChild(page, "settingsMpvConfEdit")
        verify(toggle !== null, "有 embed-mpv.conf 开关")
        verify(edit !== null, "有编辑 embed-mpv.conf 按钮")
        compare(toggle.checked, true, "默认读取 embed-mpv.conf")

        toggle.checked = false
        toggle.toggled()
        wait(40)
        compare(backend.playerMpvConf, false, "关掉后写回后端")

        edit.clicked()
        wait(40)
        compare(backend.openMpvConfCalls, 1, "编辑按钮调用后端打开文件")
    }

    function test_embed_input_conf_is_wired() {
        openSection(page.playbackSection)

        var toggle = findChild(page, "settingsInputConfEnabled")
        var edit = findChild(page, "settingsInputConfEdit")
        verify(toggle !== null, "有 embed-input.conf 开关")
        verify(edit !== null, "有编辑 embed-input.conf 按钮")
        compare(toggle.checked, true, "默认加载 embed-input.conf")

        toggle.checked = false
        toggle.toggled()
        wait(40)
        compare(backend.playerInputConf, false, "关掉后写回后端")

        edit.clicked()
        wait(40)
        compare(backend.openInputConfCalls, 1, "编辑按钮调用后端打开文件")
    }

    function test_vulkan_toggle_is_wired() {
        openSection(page.playbackSection)

        var toggle = findChild(page, "settingsVulkanEnabled")
        verify(toggle !== null, "有 Vulkan 渲染开关")
        compare(toggle.checked, true, "内嵌默认使用 Vulkan")

        toggle.checked = false
        toggle.toggled()
        wait(40)
        compare(backend.playerVulkan, false, "关闭后回退 OpenGL")
    }

    function test_auto_play_next_toggle_is_wired() {
        openSection(page.playbackSection)

        var toggle = findChild(page, "settingsAutoPlayNextToggle")
        verify(toggle !== null, "有播完自动下一集开关")
        compare(toggle.checked, true, "默认开启")

        toggle.checked = false
        toggle.toggled()
        wait(40)
        compare(backend.autoPlayNext, false, "关闭后不再自动连播")
    }

    function test_download_dir_commits_when_editing_finishes() {
        openSection(page.transfersSection)

        var field = findChild(page, "settingsDownloadDir")
        verify(field !== null, "有下载目录输入框")

        field.text = "/tmp/kichi-downloads"
        field.editingFinished()
        compare(backend.downloadDir, "/tmp/kichi-downloads")
    }

    function test_directory_picker_uses_backend() {
        openSection(page.transfersSection)

        var choose = findChild(page, "settingsChooseDownloadDir")
        verify(choose !== null, "有选择目录按钮")

        mouseClick(choose)
        wait(40)

        compare(backend.chooseDownloadDirCalls, 1, "点一次只请求一次系统选择器")
    }

    function test_download_throughput_controls_are_wired() {
        openSection(page.transfersSection)

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

    function test_upload_throughput_controls_are_wired() {
        openSection(page.transfersSection)

        var parallel = findChild(page, "settingsUploadParallel")
        var rate = findChild(page, "settingsUploadRate")
        var autoResume = findChild(page, "settingsAutoResumeUploads")
        verify(parallel !== null, "有同时上传任务数")
        verify(rate !== null, "有上传全局限速")
        verify(autoResume !== null, "有上传自动续传开关")
        compare(autoResume.checked, false, "上传自动续传默认关闭")

        parallel.value = 4
        parallel.valueModified()
        rate.value = 8
        rate.valueModified()
        autoResume.checked = true
        autoResume.toggled()

        compare(backend.uploadParallel, 4)
        compare(backend.uploadRateMib, 8)
        compare(backend.autoResumeUploads, true)
    }

    function test_local_index_switch_defaults_off_and_is_wired() {
        openSection(page.generalSection)

        var box = findChild(page, "settingsLocalIndex")
        verify(box !== null, "有本地目录索引开关")
        compare(box.checked, false, "本地目录索引默认关闭")

        box.checked = true
        box.toggled()
        wait(40)
        compare(backend.localIndexEnabled, true, "打开后写回后端")
    }

    function test_settings_path_is_right_aligned() {
        openSection(page.aboutSection)

        var path = findChild(page, "settingsPath")
        verify(path !== null, "有设置文件路径")
        compare(path.horizontalAlignment, Text.AlignRight)
    }

    function test_upload_group_keeps_the_same_spacing_as_download_group() {
        openSection(page.transfersSection)

        var download = findChild(page, "settingsDownloadGroup")
        var upload = findChild(page, "settingsUploadGroup")
        verify(download !== null, "有下载设置分组")
        verify(upload !== null, "有上传设置分组")
        compare(upload.y - (download.y + download.height), 24,
                "上传分组与上一组之间必须保留 gapXl")
    }

    // 长页面（传输页）在矮窗口里仍要能滚动，且惯性中的第一次点击不能被吞掉。
    function test_first_click_after_flick_toggles_control() {
        var smallPage = pageComponent.createObject(parent, {
            width: 900,
            height: 420,
            visible: true,
            backend: backend,
            currentSection: page.transfersSection
        })
        verify(smallPage !== null)

        var scroller = findChild(smallPage, "settingsScroller")
        var box = findChild(smallPage, "settingsAutoResumeUploads")
        verify(scroller !== null && box !== null)
        verify(scroller.contentHeight > scroller.height)

        const boxY = box.mapToItem(scroller.contentItem, 0, 0).y
        const maxY = scroller.contentHeight - scroller.height
        // 居中而不是固定偏移：文案变长时控件位置会往下移。
        scroller.contentY = Math.max(0, Math.min(maxY, boxY - scroller.height / 2))
        wait(30)
        scroller.flick(0, -100)
        wait(30)
        verify(scroller.moving, "测试必须覆盖惯性滚动期间")

        const point = scroller.mapFromItem(
            box, box.width / 2, box.height / 2)
        verify(point.x >= 0 && point.x < scroller.width - 10
               && point.y >= 0 && point.y < scroller.height)
        mouseClick(scroller, point.x, point.y)
        wait(50)
        compare(backend.autoResumeUploads, true,
                "惯性后的第一次点击必须直接切换控件")

        scroller.cancelFlick()
        smallPage.destroy()
    }
}
