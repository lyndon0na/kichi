import QtQuick
import QtTest
import ".."

TestCase {
    name: "ElidedTextRows"
    when: windowShown
    width: 1000
    height: 720

    Component {
        id: harnessComponent

        Item {
            id: harness
            width: 1000
            height: 720

            property alias fileRow: fileRow
            property alias gridItem: gridItem
            property alias trashRow: trashRow
            property alias libraryRow: libraryRow
            property alias libraryRowWithPath: libraryRowWithPath
            property alias transferRow: transferRow
            property alias narrowButton: narrowButton
            property alias searchBackend: fakeSearchBackend

            QtObject {
                id: fakeSearchBackend

                property string searchStatus: "扫描完成"
                property bool searchActive: false
                property bool searchSettled: true
                property bool searchTruncated: false
                property string searchQuery: "file"
                property int cancelPlayCalls: 0
                property bool playLoading: false
                property string playLoadingId: ""
                property var playingIds: []
                property var searchResults: [{
                    id: "search-result",
                    name: "搜索结果里同样很长的文件名用于确认名称截断提示.mkv",
                    kind: "file",
                    auditFlag: false,
                    sizeText: "1.2 GB",
                    modifiedText: "2026-10-30 12:34",
                    folderText: "很长的父目录/子目录/再下一层目录/"
                                + "很长的父目录/子目录/再下一层目录/"
                                + "很长的父目录/子目录/再下一层目录/"
                                + "很长的父目录/子目录/再下一层目录/"
                                + "很长的父目录/子目录/再下一层目录",
                    folderId: "folder",
                    playable: true
                }]

                function searchOpenFolder() {}
                function searchRevealFolder() {}
                function downloadEntry() {}
                function stopSearch() {}

                function cancelPlayLoading() {
                    cancelPlayCalls += 1
                }
            }

            FileRow {
                id: fileRow
                y: 10
                width: 760
                index: -1
                rowHeight: 48
                backend: fakeSearchBackend
                entry: harness.entry("files")
                detailText: "所在文件夹：" + fakeSearchBackend.searchResults[0].folderText
                detailClickable: true
                playLoading: fakeSearchBackend.playLoading
                             && fakeSearchBackend.playLoadingId === entry.id
                playing: fakeSearchBackend.playingIds.indexOf(entry.id) >= 0
            }

            FileGridItem {
                id: gridItem
                x: 780
                y: 10
                width: 190
                index: -1
                backend: fakeSearchBackend
                entry: harness.entry("grid")
                detailText: "所在文件夹：" + fakeSearchBackend.searchResults[0].folderText
                detailClickable: true
            }

            TrashRow {
                id: trashRow
                y: 250
                width: 760
                index: -1
                rowHeight: 48
                entry: harness.entry("trash")
            }

            LibraryRow {
                id: libraryRow
                y: 560
                width: 760
                index: -1
                rowHeight: 48
                entry: harness.libraryEntry("")
            }

            LibraryRow {
                id: libraryRowWithPath
                y: 618
                width: 760
                index: -1
                rowHeight: 48
                entry: harness.libraryEntry("/父目录/子目录")
            }

            TransferMainRow {
                id: transferRow
                y: 330
                width: 900
                key: "transfer"
                name: "传输任务里同样很长的文件名用于确认整行鼠标区域不会挡住提示.mkv"
                sourceId: "source"
                direction: "download"
                phase: "running"
                stateText: "下载中"
                running: true
                progress: 42
                progressText: "42%"
                sizeText: "1.2 GB"
                detail: ""
                path: "/tmp/file.mkv"
                folder: false
                childCount: 0
                failedCount: 0
                failedDirs: 0
                depth: 0
                rowKind: "parent"
                parentKey: ""
                expandable: false
                expanded: false
            }

            KButton {
                id: narrowButton
                y: 410
                width: 72
                text: "一个很长但会截断的按钮文字"
            }

            function libraryEntry(folderPath) {
                return {
                    id: "library",
                    name: "资料库里同样很长的文件名用来确认所在文件夹提示有内容.mkv",
                    kind: "file",
                    sizeText: "1.2 GB",
                    modifiedText: "2026-10-30 12:34",
                    primaryText: "1.2 GB",
                    secondaryText: "2026-10-30 12:34",
                    playable: true,
                    folderPath: folderPath
                }
            }

            function entry(prefix) {
                return {
                    id: prefix,
                    name: prefix + " 这是一个很长很长的文件名，用来确认悬停提示只在截断时出现.mkv",
                    kind: "file",
                    sizeText: "1.2 GB",
                    modifiedText: "2026-10-30 12:34",
                    deletedText: "2026-10-30 13:00",
                    hasThumbnail: false,
                    video: false,
                    playable: true,
                    auditFlag: false
                }
            }
        }
    }

    property var harness: null

    function init() {
        harness = harnessComponent.createObject(parent)
        verify(harness !== null, "截断文字行测试宿主创建失败")
        waitForRendering(harness)
    }

    function cleanup() {
        if (harness)
            harness.destroy()
        harness = null
    }

    function hoverCenter(container, item) {
        const point = item.mapToItem(container, item.width / 2, item.height / 2)
        mouseMove(container, point.x, point.y)
        wait(50)
    }

    function disablePopup(label) {
        verify(label !== null)
        label.toolTipEnabled = false
    }

    function test_file_row_uses_external_hover() {
        const label = findChild(harness.fileRow, "fileNameText")
        disablePopup(label)
        verify(label.truncated)
        verify(!label.toolTipRequested)

        hoverCenter(harness.fileRow, label)
        verify(label.toolTipRequested)
    }

    function test_grid_item_uses_external_hover() {
        const label = findChild(harness.gridItem, "gridName")
        disablePopup(label)
        verify(label.truncated)
        verify(!label.toolTipRequested)

        hoverCenter(harness.gridItem, label)
        verify(label.toolTipRequested)
    }

    function test_trash_row_uses_external_hover() {
        const label = findChild(harness.trashRow, "trashNameText")
        disablePopup(label)
        verify(label.truncated)
        verify(!label.toolTipRequested)

        hoverCenter(harness.trashRow, label)
        verify(label.toolTipRequested)
    }

    function test_library_row_without_folder_path_falls_back_to_name() {
        const label = findChild(harness.libraryRow, "libraryNameText")
        disablePopup(label)
        verify(label.truncated)
        verify(!label.toolTipRequested)

        hoverCenter(harness.libraryRow, label)
        verify(label.toolTipRequested)
        compare(label.toolTipText, label.text)
    }

    function test_library_row_with_folder_path_uses_folder_hint() {
        const label = findChild(harness.libraryRowWithPath, "libraryNameText")
        disablePopup(label)
        verify(label.truncated)

        hoverCenter(harness.libraryRowWithPath, label)
        verify(label.toolTipRequested)
        verify(label.toolTipText.indexOf("所在文件夹：") === 0)
    }

    function test_transfer_name_uses_row_hover_for_plain_file() {
        const label = findChild(harness.transferRow, "nameText")
        const rowHeader = findChild(harness.transferRow, "rowHeader")
        verify(rowHeader !== null)
        verify(!rowHeader.enabled)
        disablePopup(label)
        verify(label.truncated)
        verify(!label.toolTipRequested)

        hoverCenter(harness.transferRow, label)
        verify(label.toolTipRequested)
    }

    function test_button_text_uses_local_hover() {
        const label = findChild(harness.narrowButton, "buttonText")
        disablePopup(label)
        verify(label.truncated)
        verify(!label.toolTipRequested)

        hoverCenter(harness.narrowButton, label)
        verify(label.toolTipRequested)

        harness.narrowButton.showTextToolTip = false
        verify(!label.toolTipEnabled)
    }

    function test_shared_list_detail_uses_external_hover() {
        const label = findChild(harness.fileRow, "fileRowDetail")
        disablePopup(label)
        verify(label.truncated)
        verify(!label.toolTipRequested)

        hoverCenter(harness.fileRow, label)
        verify(label.toolTipRequested)
    }

    function test_shared_grid_detail_uses_external_hover() {
        const label = findChild(harness.gridItem, "gridDetail")
        disablePopup(label)
        verify(label.truncated)
        verify(!label.toolTipRequested)

        hoverCenter(harness.gridItem, label)
        verify(label.toolTipRequested)
    }

    function test_search_result_loading_uses_shared_row() {
        harness.searchBackend.playLoading = true
        harness.searchBackend.playLoadingId = "files"
        wait(30)

        const overlay = findChild(harness.fileRow, "rowVideoLoading")
        verify(overlay !== null)
        const spinner = findChild(overlay, "videoLoadingSpinner")
        const cancel = findChild(overlay, "videoLoadingCancel")
        verify(spinner !== null && cancel !== null)
        compare(overlay.visible, true)
        compare(cancel.visible, true)
        mouseClick(cancel)
        compare(harness.searchBackend.cancelPlayCalls, 1)
    }

    function test_search_result_playing_uses_shared_row_status() {
        harness.searchBackend.playingIds = ["other", "files"]
        wait(30)

        const status = findChild(harness.fileRow, "rowPlayingStatus")
        const bars = findChild(harness.fileRow, "rowPlayingBars")
        verify(status !== null && bars !== null)
        compare(status.visible, true)
        compare(bars.running, true)

        harness.searchBackend.playLoading = true
        harness.searchBackend.playLoadingId = "files"
        tryCompare(status, "visible", false)
        compare(findChild(harness.fileRow, "rowVideoLoading").visible, true)
    }
}
