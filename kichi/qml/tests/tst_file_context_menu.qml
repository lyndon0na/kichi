import QtQuick
import QtTest
import "../files"

// 文件条目右键菜单的「下载到本地」。
//
// 锁住多选行为：多选时这一项必须出现、文案带条数、点下去把**整组 id**
// 一次交给后端（`downloadEntries`），而不是只下右键点到的那一条。
// 单条仍走 `downloadEntry`（它带「已在队列中 → 跳去传输页」的语义）。
//
// 运行：
//   QT_QPA_PLATFORM=offscreen /usr/lib64/qt6/bin/qmltestrunner \
//       -input kichi/qml/tests/tst_file_context_menu.qml
TestCase {
    name: "FileContextMenu"
    when: windowShown
    width: 900
    height: 600

    QtObject {
        id: backend
        property bool filesLoading: false
        property bool fileActionRunning: false
        property var fileActionIds: []
        property bool trashClearing: false
        property bool shareActionRunning: false
        property var playMenu: []
        property var pending: []
        property int singleCount: 0
        property string singleId: ""
        property int batchCount: 0
        property var batchIds: []
        property int starCount: 0
        property var starIds: []
        property bool starValue: false
        property int copyLinkCalls: 0
        property string copyLinkId: ""
        property int propertiesCount: 0
        property string propertiesId: ""
        property int chooseDirCount: 0
        property var chooseDirIds: []

        signal changed()
        signal fileActionDone(string action, bool ok, string message, var ids)

        function isDownloadPending(id) {
            return pending.indexOf(id) >= 0
        }
        function hasPartialDownload(id) {
            return false
        }
        function downloadEntry(id) {
            singleCount += 1
            singleId = id
        }
        function downloadEntries(ids) {
            batchCount += 1
            batchIds = ids.slice()
        }
        function setStarred(ids, starred) {
            starCount += 1
            starIds = ids.slice()
            starValue = starred
        }
        function copyLink(id) {
            copyLinkCalls += 1
            copyLinkId = id
        }
        function requestEntryProperties(id) {
            propertiesCount += 1
            propertiesId = id
        }
        function chooseDownloadDirForEntries(ids) {
            chooseDirCount += 1
            chooseDirIds = ids.slice()
        }
    }

    Component {
        id: menuComponent
        FilesPageMenu {}
    }

    property var menu: null

    function init() {
        backend.fileActionIds = []
        backend.trashClearing = false
        backend.singleCount = 0
        backend.singleId = ""
        backend.batchCount = 0
        backend.batchIds = []
        backend.starCount = 0
        backend.starIds = []
        backend.starValue = false
        backend.copyLinkCalls = 0
        backend.copyLinkId = ""
        backend.propertiesCount = 0
        backend.propertiesId = ""
        backend.chooseDirCount = 0
        backend.chooseDirIds = []
        backend.pending = []
        menu = menuComponent.createObject(parent, {
            width: 600, height: 400, visible: true, backend: backend
        })
        verify(menu !== null, "FilesPageMenu 创建失败")
    }

    function cleanup() {
        if (menu)
            menu.destroy()
        menu = null
    }

    function openMenu(ids) {
        menu.openContext({ id: ids[0], name: "a.mkv", kind: "file", playable: false },
                         Qt.point(20, 20), ids, false)
        wait(60)
        var item = findChild(menu, "downloadItem")
        verify(item !== null, "菜单里有下载项")
        return item
    }

    function test_multi_select_downloads_every_selected_id() {
        var item = openMenu(["a", "b", "c"])
        verify(item.visible, "多选时下载项可见")
        verify(item.text.indexOf("3 项") >= 0, "文案带条数，实际：" + item.text)

        mouseClick(item)
        wait(40)
        compare(backend.batchCount, 1, "多选一次交给后端")
        compare(backend.batchIds.length, 3, "整组 id 都交出去")
        compare(backend.singleCount, 0, "多选不走单条入口")
    }

    function test_single_select_keeps_single_entry() {
        var item = openMenu(["a"])
        verify(item.visible, "单条时下载项可见")
        compare(item.text, qsTr("下载到本地"))

        mouseClick(item)
        wait(40)
        compare(backend.singleCount, 1, "单条走 downloadEntry")
        compare(backend.singleId, "a")
        compare(backend.batchCount, 0, "单条不调批量入口")
    }

    function test_download_to_is_offered_for_a_single_file() {
        openMenu(["a"])
        var item = findChild(menu, "downloadToItem")
        verify(item !== null, "菜单里有「下载到…」")
        verify(item.visible, "单条文件可下载到自选目录")
        compare(item.text, qsTr("下载到…"))

        mouseClick(item)
        wait(40)
        compare(backend.chooseDirCount, 1, "点一次只请求一次系统选择器")
        compare(backend.chooseDirIds.length, 1)
        compare(backend.chooseDirIds[0], "a")
        compare(backend.singleCount, 0, "不经过全局目录的下载入口")
        compare(backend.batchCount, 0)
    }

    function test_download_to_is_offered_for_a_folder() {
        menu.openContext({ id: "dir", name: "dir", kind: "folder" },
                         Qt.point(20, 20), ["dir"], false)
        wait(60)
        var item = findChild(menu, "downloadToItem")
        verify(item.visible, "文件夹也能下载到自选目录")
        compare(item.text, qsTr("下载到…"))
    }

    function test_download_to_takes_the_whole_multi_selection() {
        openMenu(["a", "b", "c"])
        var item = findChild(menu, "downloadToItem")
        verify(item.visible, "多选时「下载到…」可见")
        verify(item.text.indexOf("3 项") >= 0, "文案带条数，实际：" + item.text)

        mouseClick(item)
        wait(40)
        compare(backend.chooseDirCount, 1, "多选只弹一次选择器")
        compare(backend.chooseDirIds.length, 3, "整组 id 一次交给后端")
    }

    function test_download_to_is_hidden_in_trash() {
        menu.openContext({ id: "a", name: "a.mkv", kind: "file", inTrash: true },
                         Qt.point(20, 20), ["a"], false)
        wait(60)
        verify(!findChild(menu, "downloadToItem").visible, "回收站不提供下载到…")
    }

    function test_download_to_is_hidden_for_a_pending_single_entry() {
        backend.pending = ["a"]
        var item = openMenu(["a"])
        compare(item.text, qsTr("已在下载队列中，去查看"),
                "单条已在队列时「下载到本地」负责去查看")
        verify(!findChild(menu, "downloadToItem").visible,
               "单条已在队列时不再显示重复的「下载到…」入口")
    }

    function test_other_entry_action_does_not_gray_this_entry() {
        backend.fileActionIds = ["b"]
        menu.openContext({ id: "a", name: "a.mkv", kind: "file", playable: false },
                         Qt.point(20, 20), ["a"], false)
        wait(60)
        var star = findChild(menu, "starItem")
        var del = findChild(menu, "deleteItem")
        verify(star !== null && star.enabled, "别的条目在写，不应禁用当前条目的星标")
        verify(del !== null && del.enabled, "别的条目在写，不应禁用当前条目的删除")
    }

    function test_same_entry_action_disables_that_entry() {
        backend.fileActionIds = ["a"]
        menu.openContext({ id: "a", name: "a.mkv", kind: "file", playable: false },
                         Qt.point(20, 20), ["a"], false)
        wait(60)
        var star = findChild(menu, "starItem")
        verify(star !== null && !star.enabled, "同一目标在写时只禁用这一项")
    }

    function test_clear_in_progress_only_disables_delete() {
        backend.trashClearing = true
        menu.openContext({ id: "a", name: "a.mkv", kind: "file", playable: false },
                         Qt.point(20, 20), ["a"], false)
        wait(60)
        var star = findChild(menu, "starItem")
        var del = findChild(menu, "deleteItem")
        verify(star !== null && star.enabled, "清空回收站不应冻结文件页其它操作")
        verify(del !== null && !del.enabled, "清空期间不能把新条目移入回收站")
    }

    function test_play_label_does_not_name_the_player() {
        menu.openContext({ id: "video", name: "a.mkv", kind: "file", playable: true },
                         Qt.point(20, 20), ["video"], false)
        wait(60)

        var item = findChild(menu, "playItem")
        verify(item !== null, "菜单里有播放项")
        verify(item.visible, "可播放文件应显示播放项")
        compare(item.text, qsTr("播放"))
    }

    function test_pending_single_entry_invites_you_to_transfers() {
        backend.pending = ["a"]
        var item = openMenu(["a"])
        compare(item.text, qsTr("已在下载队列中，去查看"))
    }

    function test_share_keeps_the_whole_multi_selection() {
        var ids = ["a", "b", "c"]
        openMenu(ids)
        var item = findChild(menu, "shareItem")
        verify(item !== null, "菜单里有分享项")
        verify(item.text.indexOf("3 项") >= 0, "文案带条数，实际：" + item.text)

        var spy = Qt.createQmlObject(
            "import QtTest; SignalSpy { signalName: 'shareRequested' }",
            menu)
        spy.target = menu
        mouseClick(item)
        wait(40)
        compare(spy.count, 1)
        compare(spy.signalArguments[0][1].length, 3)
        spy.destroy()
    }

    function test_star_applies_to_the_whole_selection() {
        var ids = ["a", "b", "c"]
        openMenu(ids)
        var item = findChild(menu, "starItem")
        verify(item !== null, "菜单里有星标项")
        verify(item.text.indexOf("3 项") >= 0, "文案带条数，实际：" + item.text)

        mouseClick(item)
        wait(40)
        compare(backend.starCount, 1)
        compare(backend.starIds.length, 3)
        compare(backend.starValue, true)
    }

    function test_starred_entry_offers_unstar() {
        menu.openContext({
            id: "a",
            name: "a.mkv",
            kind: "file",
            playable: false,
            starred: true
        }, Qt.point(20, 20), ["a", "b"], false)
        wait(60)

        var item = findChild(menu, "starItem")
        verify(item.text.indexOf("取消") === 0, "实际：" + item.text)
        mouseClick(item)
        wait(40)
        compare(backend.starCount, 1)
        compare(backend.starIds.length, 2)
        compare(backend.starValue, false)
    }

    function test_search_result_offers_open_containing_directory() {
        var entry = {
            id: "result-file",
            name: "a.mkv",
            kind: "file",
            playable: false,
            folderId: "folder-2"
        }
        menu.openContext(entry, Qt.point(20, 20), [entry.id], true)
        wait(60)
        var item = findChild(menu, "revealFolderItem")
        verify(item !== null, "菜单里有「打开所在目录」")
        verify(item.visible, "搜索结果应显示「打开所在目录」")
        compare(item.text, qsTr("打开所在目录"))

        var spy = Qt.createQmlObject(
            "import QtTest; SignalSpy { signalName: 'revealFolderRequested' }", menu)
        spy.target = menu
        mouseClick(item)
        wait(40)
        compare(spy.count, 1)
        compare(String(spy.signalArguments[0][0].id), "result-file")
        spy.destroy()
    }

    function test_open_containing_directory_is_hidden_outside_search_and_trash() {
        menu.openContext({ id: "a", name: "a.mkv", kind: "file" },
                         Qt.point(20, 20), ["a"], false)
        wait(60)
        var item = findChild(menu, "revealFolderItem")
        verify(item !== null, "菜单里有「打开所在目录」")
        verify(!item.visible, "当前目录里的条目不需要「打开所在目录」")

        menu.openContext({ id: "b", name: "b.mkv", kind: "file", inTrash: true },
                         Qt.point(20, 20), ["b"], true)
        wait(60)
        verify(!item.visible, "回收站结果不提供跳转")
    }

    function test_copy_link_is_offered_for_a_single_file() {
        openMenu(["a"])
        var item = findChild(menu, "copyLinkItem")
        verify(item !== null, "菜单里有复制链接项")
        verify(item.visible, "单个文件应显示复制链接")
        compare(item.text, qsTr("复制下载链接"))

        mouseClick(item)
        wait(40)
        compare(backend.copyLinkCalls, 1)
        compare(backend.copyLinkId, "a")
    }

    function test_copy_link_is_hidden_for_folder() {
        menu.openContext({ id: "dir", name: "dir", kind: "folder" },
                         Qt.point(20, 20), ["dir"], false)
        wait(60)
        verify(!findChild(menu, "copyLinkItem").visible, "文件夹没有直链")
    }

    function test_copy_link_is_hidden_for_multi_selection() {
        menu.openContext({ id: "a", name: "a.mkv", kind: "file" },
                         Qt.point(20, 20), ["a", "b"], false)
        wait(60)
        verify(!findChild(menu, "copyLinkItem").visible, "多选时不复制链接")
    }

    function test_properties_is_offered_for_a_single_file_or_folder() {
        menu.openContext({ id: "a", name: "a.mkv", kind: "file" },
                         Qt.point(20, 20), ["a"], false)
        wait(60)
        var item = findChild(menu, "propertiesItem")
        verify(item !== null, "菜单里有属性项")
        verify(item.visible, "单个文件应显示属性")
        compare(item.text, qsTr("属性…"))

        menu.openContext({ id: "dir", name: "dir", kind: "folder" },
                         Qt.point(20, 20), ["dir"], false)
        wait(60)
        verify(item.visible, "单个文件夹也应显示属性")

        mouseClick(item)
        wait(60)
        var dialog = findChild(menu, "propertiesDialog")
        verify(dialog !== null && dialog.opened, "点属性应打开弹窗")
        compare(backend.propertiesCount, 1)
        compare(backend.propertiesId, "dir")
    }

    function test_properties_is_hidden_for_multi_selection() {
        menu.openContext({ id: "a", name: "a.mkv", kind: "file" },
                         Qt.point(20, 20), ["a", "b"], false)
        wait(60)
        verify(!findChild(menu, "propertiesItem").visible, "多选时没有单一属性可看")
    }
}
