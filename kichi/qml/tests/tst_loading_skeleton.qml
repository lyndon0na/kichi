import QtQuick
import QtTest
import "../components"
import "../core"
import "../files"

// 空内容加载骨架的共享状态与视觉布局。
TestCase {
    name: "LoadingSkeleton"
    when: windowShown
    width: 760
    height: 560

    Component {
        id: gateComponent
        LoadingSkeletonGate {}
    }

    Component {
        id: densityComponent
        FileListDensity {}
    }

    Component {
        id: metricsComponent
        UiMetrics {}
    }

    Component {
        id: skeletonComponent
        LoadingSkeleton {}
    }

    QtObject {
        id: fakeGridView

        property int columns: 3
        property real cellSlotWidth: 182
        property real itemWidth: 168
        property real itemHeight: 184
        property real sideMargin: 12
        property real columnGap: 14
        property real rowGap: 18
    }

    property var gate: null
    property var density: null
    property var metrics: null
    property var skeleton: null

    function cleanup() {
        if (skeleton)
            skeleton.destroy()
        if (metrics)
            metrics.destroy()
        if (density)
            density.destroy()
        if (gate)
            gate.destroy()
        skeleton = null
        metrics = null
        density = null
        gate = null
    }

    function test_gate_shows_skeleton_for_every_empty_load() {
        gate = gateComponent.createObject(parent)
        verify(gate !== null)

        gate.active = true
        gate.loading = true
        compare(gate.showSkeleton, true, "空内容读取显示骨架")

        gate.loading = false
        compare(gate.showSkeleton, false, "读取结束收起骨架")

        gate.loading = true
        compare(gate.showSkeleton, true, "再次空内容读取仍显示骨架")

        gate.count = 2
        compare(gate.showSkeleton, false, "已有内容时刷新不覆盖旧内容")
    }

    function test_gate_requires_an_active_page() {
        gate = gateComponent.createObject(parent)
        gate.loading = true
        compare(gate.showSkeleton, false, "页面不在前台时不画骨架")

        gate.active = true
        compare(gate.showSkeleton, true, "回到前台后对当前读取显示骨架")
    }

    function test_gate_hides_skeleton_while_signed_out() {
        gate = gateComponent.createObject(parent)
        gate.active = true
        gate.loading = true
        compare(gate.showSkeleton, true)

        gate.sessionActive = false
        compare(gate.showSkeleton, false, "退出登录期间不画内容骨架")
    }

    function test_skeleton_switches_between_list_and_grid_layouts() {
        density = densityComponent.createObject(parent)
        metrics = metricsComponent.createObject(parent)
        verify(density !== null && metrics !== null)
        skeleton = skeletonComponent.createObject(parent, {
            width: 700,
            height: 480,
            visible: true,
            density: density,
            metrics: metrics,
            listTopInset: 38,
            trailingColumnWidths: [density.sizeColumnWidth,
                                   density.modifiedColumnWidth]
        })
        verify(skeleton !== null)
        wait(30)

        const list = findChild(skeleton, "listLoadingSkeleton")
        const grid = findChild(skeleton, "gridLoadingSkeleton")
        const backdrop = findChild(skeleton, "loadingSkeletonBackdrop")
        verify(list !== null && grid !== null && backdrop !== null)
        compare(backdrop.color, skeleton.theme.surface)
        compare(list.visible, true)
        compare(grid.visible, false)
        compare(list.y, 38, "列表骨架从表头下方开始")
        verify(list.children.length > 1, "列表骨架包含多行占位")

        skeleton.gridMode = true
        skeleton.gridView = fakeGridView
        wait(30)
        compare(list.visible, false)
        compare(grid.visible, true)
        verify(grid.children.length > fakeGridView.columns,
               "网格骨架按真实列数生成卡片")

        skeleton.visible = false
        wait(20)
        compare(skeleton.pulse, 1, "隐藏后立即停止在稳定透明度")
    }
}
