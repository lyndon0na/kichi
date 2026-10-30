import QtQuick
import QtTest
import ".."

// 文件 / 回收站列表的新表格语言：
// - 两页始终使用固定缩略图槽；
// - 文件夹、空路径、下载失败都退回文件图标；
// - 视频加载成功后显示播放角标；
// - 回收站保持「名称 / 删除时间 / 大小」列序。
TestCase {
    name: "FileListStyle"
    when: windowShown
    width: 760
    height: 520

    QtObject {
        id: fakeBackend

        property string cachedUri: ""
        property int requestCount: 0
        property string requestedId: ""

        signal thumbnailReady(string id, string path)

        function thumbnailPath(id) {
            return cachedUri
        }

        function requestThumbnail(id) {
            requestCount += 1
            requestedId = id
        }

        function finish(id, path) {
            thumbnailReady(id, path)
        }
    }

    Component {
        id: harnessComponent

        Item {
            FileRow {
                id: fileRow
                objectName: "fileRow"
                y: 48
                width: 620
                rowHeight: 48
                backend: fakeBackend
                entry: ({
                    id: "folder",
                    name: "Animeko-Playing",
                    kind: "folder",
                    sizeText: "—",
                    modifiedText: "",
                    hasThumbnail: false,
                    video: false,
                    playable: false,
                    auditFlag: false
                })
            }

            TrashRow {
                id: trashRow
                objectName: "trashRow"
                y: 104
                width: 620
                rowHeight: 48
                backend: fakeBackend
                entry: ({
                    id: "deleted",
                    name: "已删除的截图.png",
                    kind: "file",
                    sizeText: "2.4 MB",
                    deletedText: "2026-10-28 12:34",
                    hasThumbnail: false,
                    video: false
                })
            }
        }
    }

    Component {
        id: thumbnailComponent

        FileThumbnail {
            width: 36
            height: 24
            backend: fakeBackend
        }
    }

    Component {
        id: filesPageComponent

        FilesPage {
            width: 620
            height: 300
        }
    }

    Component {
        id: trashHeaderComponent

        TrashTableHeader {
            width: 620
        }
    }

    property var harness: null
    property var thumbnail: null
    property var trashHeader: null
    property var filesPage: null

    function cleanup() {
        if (harness)
            harness.destroy()
        if (thumbnail)
            thumbnail.destroy()
        if (trashHeader)
            trashHeader.destroy()
        if (filesPage)
            filesPage.destroy()
        harness = null
        thumbnail = null
        trashHeader = null
        filesPage = null
        fakeBackend.cachedUri = ""
        fakeBackend.requestCount = 0
        fakeBackend.requestedId = ""
    }

    function pageEntry(id, name, video, hasThumbnail) {
        return {
            id: id,
            name: name,
            kind: "file",
            sizeText: "1.0 GB",
            modifiedText: "2026-10-30 10:00",
            deletedText: "2026-10-30 11:00",
            hasThumbnail: hasThumbnail,
            video: video,
            playable: video,
            auditFlag: false
        }
    }

    function test_rows_share_fixed_thumbnail_slot_width() {
        harness = harnessComponent.createObject(parent, { visible: true })
        verify(harness !== null, "表格布局创建失败")
        wait(30)

        var fileRow = findChild(harness, "fileRow")
        verify(fileRow !== null)

        var fileSlot = findChild(fileRow, "thumbnailSlot")
        var trashRow = findChild(harness, "trashRow")
        verify(trashRow !== null)
        var trashSlot = findChild(trashRow, "thumbnailSlot")
        compare(fileSlot.width, 36)
        compare(trashSlot.width, 36)
    }

    function test_folder_and_empty_thumbnail_fall_back_to_icon() {
        thumbnail = thumbnailComponent.createObject(parent, {
            entry: {
                id: "folder-thumb",
                name: "目录",
                kind: "folder",
                sizeText: "—",
                modifiedText: "",
                hasThumbnail: false,
                video: false
            }
        })
        verify(thumbnail !== null)
        wait(30)
        compare(findChild(thumbnail, "thumbnailImage").visible, false)
        compare(findChild(thumbnail, "thumbnailFallbackIcon").visible, true)
        compare(fakeBackend.requestCount, 0)
    }

    function test_files_page_uses_thumbnails_and_zero_row_spacing() {
        filesPage = filesPageComponent.createObject(parent, { visible: true })
        verify(filesPage !== null)
        wait(20)
        compare(findChild(filesPage, "fileList").spacing, 0)
    }

    function test_failed_thumbnail_keeps_fixed_slot_and_falls_back() {
        fakeBackend.cachedUri = ""
        thumbnail = thumbnailComponent.createObject(parent, {
            entry: pageEntry("failed-thumb", "坏封面.png", false, true)
        })
        verify(thumbnail !== null)
        compare(thumbnail.entry.hasThumbnail, true)
        wait(20)
        compare(fakeBackend.requestCount, 1)
        compare(fakeBackend.requestedId, "failed-thumb")

        fakeBackend.finish("failed-thumb", "")
        wait(20)
        compare(findChild(thumbnail, "thumbnailImage").visible, false)
        compare(findChild(thumbnail, "thumbnailFallbackIcon").visible, true)
        compare(thumbnail.width, 36)
        compare(thumbnail.height, 24)
    }

    function test_video_thumbnail_shows_play_badge() {
        fakeBackend.cachedUri = Qt.resolvedUrl("../../assets/icons/hicolor/scalable/apps/kichi.svg")
        thumbnail = thumbnailComponent.createObject(parent, {
            entry: pageEntry("video-thumb", "第01话.mp4", true, true)
        })
        verify(thumbnail !== null)
        tryCompare(thumbnail, "imageReady", true, 3000)
        verify(findChild(thumbnail, "videoBadge").visible)
    }

    function test_trash_columns_keep_deleted_before_size() {
        trashHeader = trashHeaderComponent.createObject(parent, { visible: true })
        verify(trashHeader !== null)
        var deleted = findChild(trashHeader, "trashHeaderDeleted")
        var size = findChild(trashHeader, "trashHeaderSize")
        verify(deleted !== null && size !== null)
        var deletedX = deleted.mapToItem(trashHeader, 0, 0).x
        var sizeX = size.mapToItem(trashHeader, 0, 0).x
        verify(deletedX < sizeX,
               "回收站列序应为删除时间在大小之前：" + deletedX + " / " + sizeX)
    }

    function test_rows_are_flat_and_have_bottom_dividers() {
        harness = harnessComponent.createObject(parent, { visible: true })
        verify(harness !== null)
        wait(20)
        var fileRow = findChild(harness, "fileRow")
        var trashRow = findChild(harness, "trashRow")
        verify(fileRow !== null && trashRow !== null)
        compare(fileRow.background.radius, 0)
        compare(trashRow.background.radius, 0)
        verify(findChild(fileRow, "fileRowDivider") !== null)
        verify(findChild(trashRow, "trashRowDivider") !== null)
    }
}
