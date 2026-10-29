import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtTest
import ".."

// 进度条静默性回归测试（2026-10-27）。
//
// 为什么要有：breeze 风格的 ProgressBar 在宽度抖动时会逐帧刷
// `ProgressBar.qml:71: TypeError: Cannot read property 'top' of null`。
// 这个告警只出现在 `QT_QUICK_CONTROLS_STYLE=org.kde.breeze` 下，
// 默认 Basic 风格根本刷不出来——不带风格跑这条测试等于没测：
//   QT_QPA_PLATFORM=offscreen QT_QUICK_CONTROLS_STYLE=org.kde.breeze \
//       /usr/lib64/qt6/bin/qmltestrunner -input kichi/qml/tests/tst_progress_bar.qml
//
// 断言只覆盖「没换坏」的部分（尺寸、填充跟着 position 走）；
// 「不再刷告警」由 scripts/qml-lint.sh 判：它把 QWARN 当失败。
// 已验证：把组件名换回 ProgressBar，同一条测试 48 条告警 → 失败。
TestCase {
    name: "ProgressBarQuiet"
    when: windowShown
    width: 700
    height: 200

    Component {
        id: rowComponent
        Item {
            id: host
            width: 700
            height: 160
            property int tick: 0

            ColumnLayout {
                anchors.fill: parent
                spacing: 8

                RowLayout {
                    Layout.fillWidth: true
                    spacing: 10
                    KProgressBar {
                        objectName: "determinate"
                        Layout.fillWidth: true
                        from: 0; to: 100
                        value: host.tick % 100
                    }
                    Label {
                        text: host.tick % 3 === 0 ? "100%" : (host.tick % 3 === 1 ? "9%" : "55%")
                        font.pixelSize: 12
                    }
                }

                RowLayout {
                    Layout.fillWidth: true
                    spacing: 10
                    KProgressBar {
                        objectName: "indeterminate"
                        Layout.fillWidth: true
                        from: 0; to: 100
                        value: 0
                        indeterminate: true
                    }
                    Label {
                        text: host.tick % 2 ? "3.7 MB/s (2.1%)" : "12.4 MB/s"
                        font.pixelSize: 12
                    }
                }

                RowLayout {
                    Layout.fillWidth: true
                    spacing: 10
                    KProgressBar {
                        objectName: "toggling"
                        Layout.fillWidth: true
                        from: 0; to: 100
                        value: host.tick % 100
                        indeterminate: host.tick % 2 === 0
                    }
                    Label { text: host.tick % 2 ? "100%" : "9%"; font.pixelSize: 12 }
                }
            }

            Timer { interval: 40; repeat: true; running: true; onTriggered: host.tick++ }
        }
    }

    function findBars(host) {
        var found = []
        function scan(item) {
            if (!item)
                return
            if (item.objectName === "determinate" || item.objectName === "indeterminate"
                || item.objectName === "toggling")
                found.push(item)
            var kids = item.children || []
            for (var i = 0; i < kids.length; i++)
                scan(kids[i])
        }
        scan(host)
        return found
    }

    function test_width_churn_is_quiet() {
        var host = rowComponent.createObject(null)
        verify(host !== null, "行组件创建失败")
        wait(800)

        var bars = findBars(host)
        compare(bars.length, 3, "三条进度条都要在")
        for (var i = 0; i < bars.length; i++) {
            verify(bars[i].background !== null, bars[i].objectName + " 缺少轨道")
            verify(bars[i].width > 0, bars[i].objectName + " 宽度为 0")
            verify(bars[i].height > 0, bars[i].objectName + " 高度为 0")
            verify(bars[i].contentItem.height > 0, bars[i].objectName + " 内容高度为 0")
            var groove = bars[i].background.children[0]
            var fill = bars[i].contentItem.children[0]
            verify(groove !== null, bars[i].objectName + " 轨道子项缺失")
            verify(fill !== null, bars[i].objectName + " 填充子项缺失")
            compare(groove.height, bars[i].grooveHeight,
                    bars[i].objectName + " 轨道高度不固定")
            compare(fill.height, bars[i].grooveHeight,
                    bars[i].objectName + " 填充高度不固定")
            fuzzyCompare(groove.y + groove.height / 2, bars[i].height / 2, 0.01,
                         bars[i].objectName + " 轨道没有垂直居中")
            fuzzyCompare(fill.y + fill.height / 2, bars[i].height / 2, 0.01,
                         bars[i].objectName + " 填充没有垂直居中")
        }

        var determinate = bars[0]
        determinate.value = 50
        wait(50)
        var fill = determinate.contentItem.children[0]
        verify(fill.width > determinate.width * 0.3,
               "填充宽度没跟上 position：" + fill.width + " / " + determinate.width)

        host.destroy()
    }
}
