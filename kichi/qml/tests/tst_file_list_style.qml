import QtQuick
import QtTest
import "../core"
import "../files"
import "../trash"

// 文件 / 回收站列表的新表格语言：
// - 两页始终使用固定缩略图槽；
// - 文件夹、空路径、下载失败都退回文件图标；
// - 视频保留常驻播放角标，播放加载时缩略图可取消；
// - 回收站保持「名称 / 删除时间 / 大小」列序。
TestCase {
    name: "FileListStyle"
    when: windowShown
    width: 760
    height: 520

    KTheme {
        id: themeProbe
    }

    QtObject {
        id: fakeBackend

        property string cachedUri: ""
        property int requestCount: 0
        property string requestedId: ""
        property bool playLoading: false
        property string playLoadingId: ""
        property var playingIds: []
        property int cancelPlayCalls: 0

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

        function cancelPlayLoading() {
            cancelPlayCalls += 1
            playLoading = false
            playLoadingId = ""
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
                index: -1
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
                index: -1
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
        fakeBackend.playLoading = false
        fakeBackend.playLoadingId = ""
        fakeBackend.playingIds = []
        fakeBackend.cancelPlayCalls = 0
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

    function test_file_row_reserves_a_fixed_star_slot() {
        harness = harnessComponent.createObject(parent, { visible: true })
        verify(harness !== null, "表格布局创建失败")
        wait(30)

        var row = findChild(harness, "fileRow")
        verify(row !== null)
        var slot = findChild(row, "fileRowStarSlot")
        var star = findChild(row, "fileRowStar")
        verify(slot !== null && star !== null)
        var width = slot.width

        row.entry = {
            id: "starred",
            name: "已加星标.mkv",
            kind: "file",
            sizeText: "1.0 GB",
            modifiedText: "2026-10-30 10:00",
            hasThumbnail: false,
            video: false,
            playable: false,
            auditFlag: false,
            starred: true
        }
        wait(20)
        compare(slot.width, width)
        compare(star.visible, true)
        compare(star.name, "star")
        compare(star.filled, true)
        compare(star.color, themeProbe.accent)

        row.entry = {
            id: "plain",
            name: "普通.mkv",
            kind: "file",
            sizeText: "1.0 GB",
            modifiedText: "2026-10-30 10:00",
            hasThumbnail: false,
            video: false,
            playable: false,
            auditFlag: false,
            starred: false
        }
        wait(20)
        compare(slot.width, width, "取消星标后槽位宽度不变")
        compare(star.visible, false)
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
        const image = findChild(thumbnail, "thumbnailImage")
        const effect = findChild(thumbnail, "thumbnailEffect")
        const maskShape = findChild(thumbnail, "thumbnailMaskShape")
        verify(image !== null && effect !== null && maskShape !== null)
        compare(image.visible, false)
        compare(effect.visible, false)
        compare(maskShape.radius, 3)
        compare(maskShape.width, thumbnail.width)
        compare(maskShape.height, thumbnail.height)
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
        compare(findChild(thumbnail, "thumbnailImage").visible, false)
        compare(findChild(thumbnail, "thumbnailEffect").visible, true)
        compare(findChild(thumbnail, "thumbnailEffect").maskEnabled, true)
        compare(findChild(thumbnail, "thumbnailEffect").maskSource,
                findChild(thumbnail, "thumbnailMask"))
        compare(findChild(thumbnail, "thumbnailMaskShape").radius, 3)
        const badge = findChild(thumbnail, "videoBadge")
        verify(badge !== null)
        compare(badge.visible, true)
        compare(badge.width, 16)
        compare(badge.radius, 4)
        compare(badge.color, themeProbe.mediaBadgeSoft)
    }

    function test_playable_video_without_thumbnail_keeps_play_badge() {
        thumbnail = thumbnailComponent.createObject(parent, {
            entry: pageEntry("video-no-thumb", "没有封面的第02话.mp4", true, false)
        })
        verify(thumbnail !== null)
        wait(20)

        compare(thumbnail.imageReady, false)
        compare(findChild(thumbnail, "thumbnailFallbackIcon").visible, true)
        compare(findChild(thumbnail, "videoBadge").visible, true)
    }

    function test_music_icon_shows_play_badge() {
        thumbnail = thumbnailComponent.createObject(parent, {
            entry: {
                id: "music-thumb",
                name: "夜曲.flac",
                kind: "file",
                sizeText: "42 MB",
                modifiedText: "",
                hasThumbnail: false,
                video: false,
                playable: true,
                auditFlag: false
            }
        })
        verify(thumbnail !== null)
        wait(20)

        compare(thumbnail.imageReady, false)
        const icon = findChild(thumbnail, "thumbnailFallbackIcon")
        compare(icon.visible, true)
        compare(icon.iconName, "music")
        compare(findChild(thumbnail, "videoBadge").visible, true)
    }

    function test_playing_row_uses_ring_and_name_status_without_resizing() {
        harness = harnessComponent.createObject(parent, { visible: true })
        verify(harness !== null)
        wait(20)

        const row = findChild(harness, "fileRow")
        row.entry = pageEntry("video-playing", "第03话.mp4", true, false)
        const sizeLabel = findChild(row, "fileSizeText")
        const sizeX = sizeLabel.mapToItem(row, 0, 0).x
        row.playing = true
        wait(30)

        const thumbnail = findChild(row, "fileThumbnail")
        const status = findChild(row, "rowPlayingStatus")
        const ring = findChild(thumbnail, "playingRing")
        const nameLabel = findChild(row, "fileNameText")
        verify(thumbnail !== null && status !== null && ring !== null && nameLabel !== null)
        compare(row.implicitHeight, 48)
        compare(findChild(row, "thumbnailSlot").width, 36)
        compare(sizeLabel.mapToItem(row, 0, 0).x, sizeX, "播放状态不能挤动大小列")
        compare(status.x, nameLabel.width + 4, "短文件名后的状态要紧贴名称")
        verify(status.mapToItem(row, 0, 0).x < sizeX)
        compare(thumbnail.playing, true)
        compare(findChild(thumbnail, "videoBadge").visible, false)
        compare(ring.visible, true)
        compare(ring.border.width, 2)
        compare(status.visible, true)
    }

    function test_loading_video_row_cancels_without_selecting() {
        harness = harnessComponent.createObject(parent, { visible: true })
        verify(harness !== null)
        wait(20)

        const row = findChild(harness, "fileRow")
        row.entry = pageEntry("video-loading", "第01话.mp4", true, false)
        row.playLoading = true
        row.playing = true
        wait(20)

        const overlay = findChild(row, "rowVideoLoading")
        const cancel = findChild(overlay, "videoLoadingCancel")
        const scrim = findChild(overlay, "videoLoadingScrim")
        const thumbnail = findChild(row, "fileThumbnail")
        verify(overlay !== null && cancel !== null && scrim !== null && thumbnail !== null)
        compare(overlay.visible, true)
        compare(overlay.z > thumbnail.z, true, "加载态必须盖住缩略图角标")
        compare(scrim.color, themeProbe.mediaScrimStrong)
        compare(thumbnail.playing, false)
        compare(findChild(thumbnail, "videoBadge").visible, false)
        compare(findChild(row, "rowPlayingStatus").visible, false)
        compare(findChild(row, "rowPlayCancel"), null)

        const spy = Qt.createQmlObject(
            "import QtTest; SignalSpy { signalName: 'selectRequested' }",
            row)
        spy.target = row
        mouseClick(cancel)
        compare(fakeBackend.cancelPlayCalls, 1)
        compare(spy.count, 0, "取消加载不能顺带选中或播放视频")
        spy.destroy()
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
