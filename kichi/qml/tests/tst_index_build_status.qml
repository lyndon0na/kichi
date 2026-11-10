import QtQuick
import QtTest
import "../components"

// 底栏建库状态：只在建库中占位，进度不确定（官方不给总数），
// 点「取消」要把请求交给后端，而不是让用户跑去设置里关总开关。
TestCase {
    name: "IndexBuildStatus"
    when: windowShown
    width: 480
    height: 120

    QtObject {
        id: fakeBackend

        property bool indexBuilding: false
        property int cancelCalls: 0

        function cancelLocalIndexBuild() {
            cancelCalls += 1
        }
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
        fakeBackend.cancelCalls = 0
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

    function test_cancel_calls_backend() {
        fakeBackend.indexBuilding = true
        wait(0)
        const button = findChild(status, "indexBuildCancel")
        verify(button !== null, "缺少取消入口")
        mouseClick(button)
        compare(fakeBackend.cancelCalls, 1)
    }
}
