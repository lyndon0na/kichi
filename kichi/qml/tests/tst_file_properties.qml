import QtQuick
import QtTest
import "../files"

// 文件 / 文件夹属性弹窗。
//
// 锁住：打开时先用列表字段立即显示、只请求一次详情；详情补齐缺失字段与路径；
// 失败保留已有字段并显示原因；文件夹大小不编造；异步结果只认当前条目。
//
// 运行：
//   QT_QPA_PLATFORM=offscreen /usr/lib64/qt6/bin/qmltestrunner \
//       -input kichi/qml/tests/tst_file_properties.qml
TestCase {
    name: "FilePropertiesDialog"
    when: windowShown
    width: 900
    height: 600

    // 替代 Kichi 桥接对象：只提供属性弹窗用到的信号 / 方法。
    QtObject {
        id: backend
        property var requestedIds: []
        property var breadcrumb: ["全部文件", "Movies"]

        signal entryPropertiesReady(string id, bool ok, string message, string name,
                                    string kind, string size, string created,
                                    string modified, string entryId, string path)

        function requestEntryProperties(id) {
            requestedIds = requestedIds.concat([id])
        }

        function respondOk(id, name, kind, size, created, modified, entryId, path) {
            entryPropertiesReady(id, true, "", name, kind, size, created, modified,
                                 entryId, path)
        }

        function respondFailure(id, message) {
            entryPropertiesReady(id, false, message, "", "", "", "", "", "", "")
        }
    }

    Component {
        id: dialogComponent
        FilePropertiesDialog {}
    }

    property var page: null

    function init() {
        backend.requestedIds = []
        page = dialogComponent.createObject(parent, { backend: backend })
        verify(page !== null, "FilePropertiesDialog 创建失败")
    }

    function cleanup() {
        if (page)
            page.destroy()
        page = null
    }

    function sampleEntry() {
        return {
            id: "f1",
            name: "movie.mkv",
            kind: "file",
            sizeText: "1.5 GB",
            createdText: "",
            modifiedText: "2026-11-01 10:00"
        }
    }

    function openDialog(entry, fromSearch, pathHint) {
        page.openFor(entry, fromSearch === true, pathHint === undefined ? "" : pathHint)
        wait(60)
        var dialog = findChild(page, "propertiesDialog")
        verify(dialog !== null && dialog.opened, "属性弹窗打开了")
        return dialog
    }

    function test_open_shows_cached_fields_and_requests_detail_once() {
        var dialog = openDialog(sampleEntry())

        compare(dialog.nameText, "movie.mkv")
        compare(dialog.kindText, qsTr("文件"))
        compare(dialog.sizeText, "1.5 GB")
        compare(dialog.createdText, "—", "缺失的创建时间显示破折号")
        compare(dialog.modifiedText, "2026-11-01 10:00")
        compare(dialog.entryIdText, "f1")
        compare(dialog.pathText, "全部文件 / Movies / movie.mkv")
        verify(dialog.loading, "详情到达前处于加载状态")
        verify(findChild(dialog, "propertiesLoading").visible, "加载提示可见")

        compare(backend.requestedIds.length, 1, "打开一次只请求一次详情")
        compare(backend.requestedIds[0], "f1")
    }

    function test_complete_entry_needs_no_detail_request() {
        var dialog = openDialog({
            id: "f2",
            name: "done.mkv",
            kind: "file",
            sizeText: "2.0 GB",
            createdText: "2026-10-01 08:30",
            modifiedText: "2026-11-01 10:00"
        }, true, "全部文件 / Movies / done.mkv")

        compare(dialog.pathText, "全部文件 / Movies / done.mkv", "调用页给的路径优先")
        compare(dialog.createdText, "2026-10-01 08:30")
        compare(dialog.loading, false, "字段齐全时不用等详情")
        verify(!findChild(dialog, "propertiesLoading").visible)
        compare(backend.requestedIds.length, 0, "字段齐全时不为弹窗多发请求")
    }

    function test_unknown_kind_is_not_guessed() {
        // 离线任务只给产物 id、不给类型时，先显示破折号，等详情补
        var dialog = openDialog({
            id: "task-file",
            name: "产物.mkv",
            kind: "",
            sizeText: "",
            createdText: "",
            modifiedText: ""
        }, true)
        compare(dialog.kindText, "—")
        compare(dialog.sizeText, "—")
        compare(backend.requestedIds.length, 1)
    }

    function test_detail_fills_missing_fields_and_path() {
        var dialog = openDialog(sampleEntry(), true)
        compare(dialog.pathText, "—", "搜索场景初始路径未知时显示破折号")

        backend.respondOk("f1", "movie.mkv", "文件", "1,536 B",
                          "2026-10-01 08:30", "2026-11-02 09:15",
                          "cloud-id-f1", "全部文件 / Movies / movie.mkv")
        wait(40)

        compare(dialog.loading, false)
        compare(dialog.errorText, "")
        compare(dialog.sizeText, "1,536 B")
        compare(dialog.createdText, "2026-10-01 08:30")
        compare(dialog.modifiedText, "2026-11-02 09:15")
        compare(dialog.entryIdText, "cloud-id-f1", "云端 id 以详情为准")
        compare(dialog.pathText, "全部文件 / Movies / movie.mkv")
        verify(!findChild(dialog, "propertiesLoading").visible, "加载提示收起")
    }

    function test_failure_keeps_cached_fields_and_shows_reason() {
        var dialog = openDialog(sampleEntry())

        backend.respondFailure("f1", "网络连接中断")
        wait(40)

        compare(dialog.loading, false)
        verify(dialog.errorText.indexOf("网络连接中断") >= 0,
               "失败原因要显示出来，实际：" + dialog.errorText)
        compare(dialog.nameText, "movie.mkv", "读取失败不清空已知字段")
        compare(dialog.modifiedText, "2026-11-01 10:00")
    }

    function test_folder_never_invents_a_total_size() {
        var dialog = openDialog({
            id: "dir1",
            name: "Movies",
            kind: "folder",
            sizeText: "—",
            createdText: "",
            modifiedText: ""
        }, true)
        compare(dialog.kindText, qsTr("文件夹"))
        compare(dialog.sizeText, "—")

        backend.respondOk("dir1", "Movies", "文件夹", "—", "", "", "dir1",
                          "全部文件 / Movies")
        wait(40)
        compare(dialog.sizeText, "—", "详情也不给文件夹总大小")
        compare(dialog.createdText, "—")
        compare(dialog.modifiedText, "—")
    }

    function test_response_for_another_entry_is_ignored() {
        var dialog = openDialog(sampleEntry())

        backend.respondOk("other-id", "别的文件", "文件", "1 B", "", "", "other-id", "/x")
        wait(40)

        compare(dialog.nameText, "movie.mkv", "别的条目结果不能覆盖当前弹窗")
        verify(dialog.loading, "当前条目仍在等待自己的结果")
    }

    function test_copy_offers_cloud_entry_id() {
        var dialog = openDialog(sampleEntry())
        backend.respondOk("f1", "movie.mkv", "文件", "1.5 GB", "", "", "cloud-id-f1", "")
        wait(40)

        var copy = findChild(dialog, "propertiesCopyId")
        verify(copy !== null && copy.enabled, "云端 id 可复制")
        mouseClick(copy)
        wait(40)
        verify(dialog.copyNotice.length > 0, "复制后给出反馈")
    }
}
