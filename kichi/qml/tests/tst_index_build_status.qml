import QtQuick
import QtTest
import "../components"

// 底栏建库状态：只在建库中占位，进度不确定（官方不给总数）。
// 全盘搜索和建库共用同一次遍历，不提供停止入口。
TestCase {
    name: "IndexBuildStatus"
    when: windowShown
    width: 480
    height: 120

    QtObject {
        id: fakeBackend

        property bool indexBuilding: false
    }

    Component {
        id: statusComponent

        IndexBuildStatus {
            backend: fakeBackend
        }
    }

    property var status: null

    function init() {
        status = statusComponent.createObject(parent)
    }

    function cleanup() {
        if (status)
            status.destroy()
        status = null
        fakeBackend.indexBuilding = false
    }

    function test_hidden_until_building() {
        compare(status.building, false)
        verify(!status.visible, "没有建库时不该占用底栏")

        fakeBackend.indexBuilding = true
        wait(0)
        verify(status.building)
        verify(status.visible, "建库中要显示进度")
    }

    function test_progress_is_indeterminate() {
        fakeBackend.indexBuilding = true
        wait(0)
        const bar = findChild(status, "indexBuildProgress")
        verify(bar !== null, "缺少进度条")
        verify(bar.visible, "建库中进度条应可见")
        verify(bar.indeterminate, "全盘建库没有总数，只能是不确定进度")
    }

    function test_has_no_stop_entry() {
        fakeBackend.indexBuilding = true
        wait(0)
        const button = findChild(status, "indexBuildCancel")
        verify(button === null, "建库与全盘扫描共用一次遍历，不应提供停止按钮")
    }
}
