import QtQuick
import QtTest
import "../library"

// 页头数据来源：只属于「已加星标」，且只在本地索引已开启、已就绪时显示。
TestCase {
    name: "LibraryHeaderControls"
    when: windowShown
    width: 480
    height: 80

    QtObject {
        id: fakeBackend

        property string contentView: "list"
        property string libraryStatus: ""
        property bool localIndexEnabled: true
    }

    QtObject {
        id: fakeTarget

        property var backend: fakeBackend
        property bool loading: false
        property string kind: "starred"

        function refresh() {}
        function setContentView(view) {}
    }

    Component {
        id: controlsComponent

        LibraryHeaderControls {
            target: fakeTarget
        }
    }

    property var controls: null

    function init() {
        controls = controlsComponent.createObject(parent, { "visible": true })
    }

    function cleanup() {
        if (controls)
            controls.destroy()
        controls = null
        fakeBackend.libraryStatus = ""
        fakeBackend.localIndexEnabled = true
        fakeTarget.kind = "starred"
    }

    function hint() {
        return findChild(controls, "librarySourceHint")
    }

    function test_starred_hint_shows_compacted_status() {
        fakeBackend.libraryStatus = "本地索引 · 更新于 11-04 10:00"
        wait(0)
        verify(hint() !== null, "缺少数据来源提示")
        verify(hint().visible)
        compare(hint().text, "本地索引 · 更新于 11-04 10:00")
    }

    function test_hint_hidden_when_index_is_disabled() {
        fakeBackend.libraryStatus = "本地索引 · 更新于 11-04 10:00"
        fakeBackend.localIndexEnabled = false
        wait(0)
        verify(!hint().visible, "索引关闭时不应解释数据来源")

        fakeBackend.localIndexEnabled = true
        wait(0)
        verify(hint().visible)
    }

    function test_hint_is_hidden_for_other_library_pages() {
        fakeBackend.libraryStatus = "本地索引 · 更新于 11-04 10:00"
        fakeTarget.kind = "recent-saved"
        wait(0)
        verify(!hint().visible, "来源提示只属于已加星标页")

        fakeTarget.kind = "starred"
        fakeBackend.libraryStatus = ""
        wait(0)
        verify(!hint().visible, "没有来源说明时不留空位")
    }
}
