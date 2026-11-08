import QtQuick
import QtQuick.Controls
import QtTest
import "../components"
import "../core"

// 回归：KDialog 不再依赖 Kirigami 的 DialogHeader。标题、关闭按钮必须
// 用应用自己的主题令牌；标题为空时不占高度。
TestCase {
    name: "DialogHeader"
    when: windowShown
    width: 500
    height: 400

    KDialog {
        id: dialog
        parent: Overlay.overlay
        anchors.centerIn: Overlay.overlay
        title: "新建任务"
        width: 360

        Label {
            text: "正文"
        }
    }

    function init() {
        dialog.title = "新建任务"
    }

    function test_header_uses_app_tokens() {
        dialog.open()
        wait(0)

        var title = findChild(dialog, "dialogTitleLabel")
        verify(title !== null, "标题 Label 必须存在")
        compare(title.text, "新建任务")
        compare(String(title.color), String(dialog.theme.textPrimary))

        var close = findChild(dialog, "dialogCloseButton")
        verify(close !== null, "关闭按钮必须存在")
        compare(close.iconName, "x")

        verify(dialog.header.implicitHeight > 0, "有标题时标题栏必须有高度")
    }

    function test_empty_title_hides_header() {
        dialog.title = ""
        wait(0)
        compare(dialog.header.implicitHeight, 0)
        verify(!dialog.header.visible)
    }
}
