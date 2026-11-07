import QtQuick
import QtQuick.Controls
import QtTest
import ".."

// 播放加载提示条的回归测试（2026-10-27）。
//
// 为什么要有：提示条是浮层，最怕写成 `anchors.fill` 的透明层——
// 看上去只是「一条提示」，实际会把整个文件列表的点击吃掉。
// 另外 400 ms 延迟、错误立即显示、取消只调一次接口，都是用户能直接看见的行为。
//
// 运行：
//   QT_QPA_PLATFORM=offscreen /usr/lib64/qt6/bin/qmltestrunner \
//       -input kichi/qml/tests/tst_play_toast.qml
TestCase {
    name: "PlayToast"
    when: windowShown
    width: 900
    height: 600

    // 替代 Kichi 桥接对象：只提供提示条用到的属性与两个入口。
    QtObject {
        id: backend
        property bool playLoading: false
        property string playLoadingName: ""
        property string playError: ""
        property string playNotice: ""
        property int cancelCalls: 0
        property int dismissCalls: 0
        property int noticeCalls: 0
        signal changed()

        function cancelPlayLoading() {
            cancelCalls += 1
            playLoading = false
            playLoadingName = ""
            changed()
        }

        function dismissPlayError() {
            dismissCalls += 1
            playError = ""
            changed()
        }

        function dismissPlayNotice() {
            noticeCalls += 1
            playNotice = ""
            changed()
        }

        function push() {
            changed()
        }
    }

    Component {
        id: toastComponent
        PlayToast {}
    }

    /// 带底栏的窗口：用来验证提示条不会压到底栏上。
    Component {
        id: footerWindowComponent
        ApplicationWindow {
            id: footerWindow
            width: 600
            height: 400
            /// 测试要量底栏的几何
            readonly property Item footerProbe: footerItem

            footer: Rectangle {
                id: footerItem
                height: 40
                color: "blue"
            }
        }
    }

    property var toast: null

    function init() {
        backend.playLoading = false
        backend.playLoadingName = ""
        backend.playError = ""
        backend.playNotice = ""
        backend.cancelCalls = 0
        backend.dismissCalls = 0
        backend.noticeCalls = 0
        // 挂在测试窗口下：布局几何要真实（尺寸断言用）。
        // ⚠ 不要在 createObject 里传 visible：那会断掉组件自己的 `visible: shown` 绑定。
        toast = toastComponent.createObject(parent, {
            backend: backend
        })
        verify(toast !== null, "PlayToast 创建失败")
    }

    function cleanup() {
        if (toast)
            toast.destroy()
        toast = null
    }

    function startLoading(name) {
        backend.playLoading = true
        backend.playLoadingName = name || "测试视频.mkv"
        backend.push()
    }

    function stopLoading() {
        backend.playLoading = false
        backend.playLoadingName = ""
        backend.push()
    }

    // 一开始没有任何播放：不能占着窗口底部吃点击。
    function test_hidden_at_start() {
        wait(50)
        compare(toast.shown, false)
        compare(toast.visible, false)
    }

    // 400 ms 内不出现（避免一闪而过），之后才出现。
    function test_loading_appears_after_delay() {
        startLoading()
        wait(150)
        compare(toast.shown, false, "400 ms 内不该出现")
        tryCompare(toast, "shown", true, 1200)
    }

    // 加载结束要真隐藏（只把 opacity 设 0 仍然会吃点击）。
    function test_loading_hides_when_done() {
        startLoading()
        tryCompare(toast, "shown", true, 1200)
        stopLoading()
        tryCompare(toast, "shown", false, 600)
        compare(toast.visible, false)
    }

    // 文件页已经在缩略图里显示加载 / 取消时，底部提示条不能再叠一层。
    function test_inline_loading_suppresses_toast() {
        toast.inlineLoading = true
        startLoading()
        wait(600)
        compare(toast.shown, false)
        compare(toast.visible, false)

        toast.inlineLoading = false
        tryCompare(toast, "shown", true, 1200)
    }

    // 点「取消」调一次接口，提示条自己收起。
    function test_cancel_button_calls_backend_once() {
        startLoading()
        tryCompare(toast, "shown", true, 1200)
        var button = findChild(toast, "playToastAction")
        verify(button !== null, "找不到提示条按钮")
        compare(button.text, "取消")
        mouseClick(button)
        compare(backend.cancelCalls, 1)
        tryCompare(toast, "shown", false, 600)
    }

    // 错误态：立即出现（不享受 400 ms 延迟），点「关闭」调对应接口。
    function test_error_shows_immediately_and_dismisses() {
        backend.playError = "播放「x」失败：链接已过期"
        backend.push()
        compare(toast.shown, true, "错误必须立即显示")
        var button = findChild(toast, "playToastAction")
        verify(button !== null, "找不到提示条按钮")
        compare(button.text, "关闭")
        mouseClick(button)
        compare(backend.dismissCalls, 1)
    }

    // 回退提示：几秒后自己收起，不需要用户点；收起时通知后端清掉文案。
    function test_notice_hides_after_a_few_seconds() {
        backend.playNotice = "「x」没有1080P，已播放原画"
        backend.push()
        compare(toast.shown, true)
        tryCompare(toast, "shown", false, 4500)
        compare(backend.noticeCalls, 1, "收起时要让后端清掉文案")
        compare(backend.playNotice, "")
    }

    // 同一份回退提示再来一次（比如重播同一个文件）要能重新弹出。
    function test_same_notice_can_show_again() {
        var text = "「x」没有1080P，已播放原画"
        backend.playNotice = text
        backend.push()
        tryCompare(toast, "shown", true, 500)
        tryCompare(toast, "shown", false, 4500)
        backend.playNotice = text
        backend.push()
        compare(toast.shown, true, "同样的文案再来一次也要显示")
    }

    // 防回归：提示条只能占自身尺寸，不能铺满窗口。
    function test_toast_does_not_cover_the_window() {
        startLoading("一个很长很长的文件名用来把提示条撑开看看会不会铺满整个窗口都不够放.mkv")
        tryCompare(toast, "shown", true, 1200)
        verify(toast.width < 600, "提示条宽度异常：" + toast.width)
        verify(toast.height < 120, "提示条高度异常：" + toast.height)
        verify(toast.width <= 900, "不能宽过窗口：" + toast.width)
    }

    // 底栏出现时，提示条要整体落在底栏上方（ApplicationWindow 的 contentItem
    // 已经把 footer 扣掉，这里把这个前提钉住）。
    function test_toast_stays_above_the_footer() {
        var win = footerWindowComponent.createObject(parent, { visible: true })
        verify(win !== null, "测试窗口创建失败")
        wait(100)
        var local = toastComponent.createObject(win.contentItem, { backend: backend })
        backend.playError = "播放失败"
        backend.push()
        wait(100)
        compare(local.shown, true)
        var bottom = local.mapToItem(null, 0, local.height).y
        var footerTop = win.footerProbe.mapToItem(null, 0, 0).y
        verify(bottom <= footerTop, "提示条压到底栏上了：" + bottom + " > " + footerTop)
        local.destroy()
        win.destroy()
    }
}
