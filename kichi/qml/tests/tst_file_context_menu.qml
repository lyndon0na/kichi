import QtQuick
import QtTest
import ".."

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
        property bool shareActionRunning: false
        property var playMenu: []
        property var pending: []
        property int singleCount: 0
        property string singleId: ""
        property int batchCount: 0
        property var batchIds: []

        signal changed()
        signal fileActionDone(string action, bool ok, string message)

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
    }

    Component {
        id: menuComponent
        FilesPageMenu {}
    }

    property var menu: null

    function init() {
        backend.singleCount = 0
        backend.singleId = ""
        backend.batchCount = 0
        backend.batchIds = []
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

    function test_context_menu_does_not_offer_open_containing_directory() {
        var entry = {
            id: "result-file",
            name: "a.mkv",
            kind: "file",
            playable: false,
            folderId: "folder-2"
        }
        menu.openContext(entry, Qt.point(20, 20), [entry.id], true)
        wait(60)
        compare(findChild(menu, "revealFolderItem"), null,
                "文件页右键菜单不应再有「打开所在目录」")
    }
}
