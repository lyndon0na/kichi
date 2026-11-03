import QtQuick
import QtTest
import ".."

// 文件页拖放上传：只负责把 URI 和拖入时的目录快照交给 Rust。
TestCase {
    name: "UploadDropArea"
    when: windowShown
    width: 640
    height: 480

    QtObject {
        id: backend

        property int calls: 0
        property string parentId: ""
        property string parentName: ""
        property var urls: []

        function reset() {
            calls = 0
            parentId = ""
            parentName = ""
            urls = []
        }

        function uploadDroppedFiles(folderId, folderName, droppedUrls) {
            calls += 1
            parentId = String(folderId || "")
            parentName = String(folderName || "")
            urls = droppedUrls.slice()
        }
    }

    Component {
        id: dropComponent

        FilesPageUploadDropArea {
            parentId: "folder-42"
            destinationName: "动画 / 本季"
        }
    }

    Component {
        id: titleComponent

        FilesPageUploadDropArea {
            acceptsDrops: true
            destinationName: ""
        }
    }

    property var dropArea: null

    function reset() {
        backend.reset()
        if (dropArea)
            dropArea.destroy()
        dropArea = null
    }

    function makeDropArea(component) {
        dropArea = component.createObject(parent, {
            backend: backend,
            acceptsDrops: true,
            width: 600,
            height: 420
        })
        verify(dropArea !== null, "拖放层创建失败")
    }

    function test_submit_forwards_directory_snapshot_and_urls() {
        reset()
        makeDropArea(dropComponent)
        dropArea.submitUrls(["file:///tmp/a%20b.mkv", "file:///tmp/c.mp4"])

        compare(backend.calls, 1)
        compare(backend.parentId, "folder-42")
        compare(backend.parentName, "动画 / 本季")
        compare(backend.urls.length, 2)
        compare(backend.urls[0], "file:///tmp/a%20b.mkv")
        compare(backend.urls[1], "file:///tmp/c.mp4")
    }

    function test_empty_urls_are_ignored() {
        reset()
        makeDropArea(dropComponent)
        dropArea.submitUrls([])
        compare(backend.calls, 0)

        dropArea.submitUrls(["", null, undefined])
        compare(backend.calls, 0)
    }

    function test_disabled_drop_does_not_submit() {
        reset()
        makeDropArea(dropComponent)
        dropArea.acceptsDrops = false
        dropArea.submitUrls(["file:///tmp/a.mkv"])
        compare(backend.calls, 0)
    }

    function test_overlay_uses_current_folder_fallback() {
        reset()
        makeDropArea(titleComponent)
        const overlay = findChild(dropArea, "uploadDropOverlay")
        const card = findChild(dropArea, "uploadDropCard")
        const title = findChild(dropArea, "uploadDropTitle")
        verify(overlay !== null && card !== null && title !== null)
        verify(overlay.color.a > 0.8, "遮罩必须压住底下的文件卡片")
        compare(card.color.a, 1.0, "提示卡必须是不透明的")
        compare(title.text, "松开以上传或添加离线任务到 当前文件夹")
    }
}
