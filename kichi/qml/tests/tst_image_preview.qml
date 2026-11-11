import QtQuick
import QtQuick.Controls
import QtTest
import "../files"

// 图片预览浮层的状态与缩放逻辑。
//
// 取图链路在 Rust（详情 → 直链 → 本地临时文件）；这里只锁 QML 侧：
// 加载态 / 失败原因 / 适应窗口 / 缩放范围 / 关闭时通知后端。
TestCase {
    name: "ImagePreviewOverlay"
    when: windowShown
    width: 800
    height: 600

    // 1×1 PNG：用于让 Image 真正进入 Ready，不依赖磁盘文件。
    readonly property string pixelPng: "data:image/png;base64,"
        + "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=="

    QtObject {
        id: backend

        property bool imagePreviewLoading: true
        property string imagePreviewPath: ""
        property string imagePreviewError: ""
        property int closeCount: 0

        function closeImagePreview() {
            closeCount += 1
        }
    }

    Component {
        id: overlayComponent
        ImagePreviewOverlay {}
    }

    property var overlay: null

    function init() {
        backend.imagePreviewLoading = true
        backend.imagePreviewPath = ""
        backend.imagePreviewError = ""
        backend.closeCount = 0
        overlay = overlayComponent.createObject(parent, {
            visible: false,
            backend: backend
        })
        verify(overlay !== null, "ImagePreviewOverlay 创建失败")
    }

    function cleanup() {
        if (overlay) {
            overlay.destroy()
            overlay = null
        }
    }

    function test_loading_then_ready_reaches_fit_scale() {
        overlay.openFor("id", "cover.png")
        verify(overlay.visible)
        verify(overlay.loading, "取图期间显示加载态")
        compare(overlay.zoom, 1)

        backend.imagePreviewLoading = false
        backend.imagePreviewPath = pixelPng
        tryVerify(function () { return overlay.imageReady }, 3000)
        verify(!overlay.loading)
        verify(!overlay.hasError)
    }

    function test_failure_reason_is_visible() {
        overlay.openFor("id", "broken.png")
        backend.imagePreviewLoading = false
        backend.imagePreviewError = "图片太大（80.0 MB），超过 64.0 MB 的预览上限"

        verify(overlay.hasError)
        verify(overlay.errorText.indexOf("预览上限") >= 0, overlay.errorText)
        const errorBlock = findChild(overlay.contentItem, "imagePreviewError")
        verify(errorBlock !== null)
        verify(errorBlock.visible)
    }

    function test_fit_button_resets_zoom_and_pan() {
        overlay.openFor("id", "cover.png")
        backend.imagePreviewLoading = false
        backend.imagePreviewPath = pixelPng
        tryVerify(function () { return overlay.imageReady }, 3000)

        overlay.zoomAt(overlay.width / 2, overlay.height / 2, 2)
        verify(overlay.zoom > 1, "滚轮 / 缩放应放大")
        overlay.panX = 40
        overlay.panY = 30
        overlay.fitToWindow()
        compare(overlay.zoom, 1)
        compare(overlay.panX, 0)
        compare(overlay.panY, 0)
    }

    function test_zoom_is_clamped_between_fit_and_eight() {
        overlay.openFor("id", "cover.png")
        backend.imagePreviewLoading = false
        backend.imagePreviewPath = pixelPng
        tryVerify(function () { return overlay.imageReady }, 3000)

        overlay.zoomAt(overlay.width / 2, overlay.height / 2, 100)
        compare(overlay.zoom, 8)
        overlay.zoomAt(overlay.width / 2, overlay.height / 2, 0.01)
        compare(overlay.zoom, 1)
    }

    function test_dismiss_notifies_the_backend() {
        overlay.openFor("id", "cover.png")
        overlay.dismiss()
        wait(50)
        compare(backend.closeCount, 1)
    }
}
