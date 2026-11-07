import QtQuick
import QtQuick.Controls
import QtTest
import "../components"

TestCase {
    name: "ElidedLabel"
    when: windowShown
    width: 360
    height: 300

    Component {
        id: harnessComponent

        Item {
            width: 360
            height: 300

            property alias truncatedText: truncatedLabel
            property alias shortText: shortLabel
            property alias externalText: externalLabel
            property alias forcedText: forcedLabel
            property alias clickableOverlayText: clickableOverlayLabel
            property alias disabledOverlayText: disabledOverlayLabel
            property alias emptyOverrideText: emptyOverrideLabel
            property alias customOverrideText: customOverrideLabel

            ElidedLabel {
                id: truncatedLabel
                objectName: "truncatedLabel"
                x: 10
                y: 10
                width: 90
                height: 24
                text: "一个足够长、必然会被截断的文件名.mkv"
                elide: Text.ElideRight
                toolTipEnabled: false
            }

            ElidedLabel {
                id: shortLabel
                objectName: "shortLabel"
                x: 10
                y: 50
                width: 300
                height: 24
                text: "短文本"
                elide: Text.ElideRight
                toolTipEnabled: false
            }

            ElidedLabel {
                id: externalLabel
                objectName: "externalLabel"
                x: 10
                y: 90
                width: 90
                height: 24
                text: "由整行鼠标区域驱动的长文件名.mkv"
                elide: Text.ElideRight
                toolTipEnabled: false
            }

            ElidedLabel {
                id: forcedLabel
                objectName: "forcedLabel"
                x: 10
                y: 130
                width: 300
                height: 24
                text: "补充提示"
                elide: Text.ElideRight
                toolTipEnabled: false
                showToolTipWhenUntruncated: true
                externallyHovered: true
            }

            ElidedLabel {
                id: clickableOverlayLabel
                x: 10
                y: 170
                width: 90
                height: 24
                text: "可点击面包屑里的长目录名"
                elide: Text.ElideMiddle
                toolTipEnabled: false
                externallyHovered: clickableOverlay.containsMouse

                MouseArea {
                    id: clickableOverlay
                    anchors.fill: parent
                    hoverEnabled: true
                }
            }

            ElidedLabel {
                id: disabledOverlayLabel
                x: 10
                y: 210
                width: 90
                height: 24
                text: "当前面包屑里的长目录名"
                elide: Text.ElideMiddle
                toolTipEnabled: false
                externallyHovered: disabledOverlay.containsMouse

                MouseArea {
                    id: disabledOverlay
                    anchors.fill: parent
                    enabled: false
                    hoverEnabled: true
                }
            }

            // 调用点把提示文字改写成空串（例如条目没有所在文件夹）时不能弹空框。
            ElidedLabel {
                id: emptyOverrideLabel
                x: 10
                y: 240
                width: 90
                height: 24
                text: "调用点改成空文本的长目录名"
                elide: Text.ElideRight
                toolTipEnabled: false
                externallyHovered: true
                ToolTip.text: ""
            }

            ElidedLabel {
                id: customOverrideLabel
                x: 10
                y: 270
                width: 90
                height: 24
                text: "调用点改成补充说明的长目录名"
                elide: Text.ElideRight
                toolTipEnabled: false
                externallyHovered: true
                ToolTip.text: "所在文件夹：/a/b/c"
            }
        }
    }

    property var harness: null

    function init() {
        harness = harnessComponent.createObject(parent)
        verify(harness !== null, "ElidedLabel 测试宿主创建失败")
    }

    function cleanup() {
        if (harness)
            harness.destroy()
        harness = null
    }

    function test_only_truncated_text_shows_after_hover() {
        verify(harness.truncatedText.truncated)
        verify(!harness.shortText.truncated)
        verify(!harness.truncatedText.toolTipRequested)

        mouseMove(harness.truncatedText, 10, 10)
        wait(50)
        verify(harness.truncatedText.toolTipRequested)

        mouseMove(harness, 330, 200)
        wait(50)
        verify(!harness.truncatedText.toolTipRequested)
    }

    function test_untruncated_text_does_not_show() {
        mouseMove(harness.shortText, 10, 10)
        wait(50)
        verify(!harness.shortText.toolTipRequested)
    }

    function test_external_hover_and_forced_tooltip() {
        verify(harness.externalText.truncated)
        verify(!harness.externalText.toolTipRequested)

        harness.externalText.externallyHovered = true
        wait(0)
        verify(harness.externalText.toolTipRequested)

        verify(harness.forcedText.toolTipRequested)
    }

    function test_tooltip_text_defaults_to_visible_text() {
        compare(harness.truncatedText.toolTipText, harness.truncatedText.text)
    }

    function test_empty_text_override_never_requests_a_tooltip() {
        verify(harness.emptyOverrideText.truncated)
        verify(!harness.emptyOverrideText.toolTipRequested)
    }

    function test_text_override_with_content_still_requests_a_tooltip() {
        verify(harness.customOverrideText.truncated)
        verify(harness.customOverrideText.toolTipRequested)
    }

    function test_breadcrumb_mouse_areas_do_not_block_tooltips() {
        verify(harness.clickableOverlayText.truncated)
        mouseMove(harness.clickableOverlayText, 10, 10)
        wait(50)
        verify(harness.clickableOverlayText.toolTipRequested)

        verify(harness.disabledOverlayText.truncated)
        mouseMove(harness.disabledOverlayText, 10, 10)
        wait(50)
        verify(harness.disabledOverlayText.toolTipRequested)
    }
}
