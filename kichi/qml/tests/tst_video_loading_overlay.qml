import QtQuick
import QtTest
import ".."

// VideoLoadingOverlay：列表里点击取消；播放窗口里把 cancellable 关掉后
// 只显示加载环和文字提示，点击不再取消。
TestCase {
    name: "VideoLoadingOverlay"
    when: windowShown
    width: 400
    height: 200

    Component {
        id: overlayComponent

        VideoLoadingOverlay {
            width: 360
            height: 180
            property int cancelCount: 0
            onCancelRequested: cancelCount += 1
        }
    }

    property var overlay: null

    function init() {
        overlay = overlayComponent.createObject(parent, { visible: true })
        verify(overlay !== null, "VideoLoadingOverlay 创建失败")
        wait(20)
    }

    function cleanup() {
        if (overlay)
            overlay.destroy()
        overlay = null
    }

    function test_click_emits_cancel_by_default() {
        mouseClick(findChild(overlay, "videoLoadingCancel"))
        compare(overlay.cancelCount, 1)
    }

    function test_non_cancellable_ignores_click() {
        overlay.cancellable = false
        wait(10)
        mouseClick(findChild(overlay, "videoLoadingCancel"))
        compare(overlay.cancelCount, 0, "不可取消时点击不能发取消请求")
    }

    function test_label_is_shown_only_when_set() {
        const label = findChild(overlay, "videoLoadingLabel")
        verify(label !== null)
        compare(label.visible, false)
        compare(label.text, "")
        overlay.label = "正在加载「测试.mkv」…"
        compare(label.visible, true)
        compare(label.text, "正在加载「测试.mkv」…")
    }
}
