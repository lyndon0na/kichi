import QtQuick
import QtQuick.Layouts
import "../components"
import "../core"

// 账号区的容量指示：已用 / 上限文字 + 铺满一行的 3px 容量条。
//
// 2026-11-05 曾整块挪到底栏右侧；2026-11-10 移回账号区——
// 底栏只剩页面上下文，侧栏底部也不会显得太空。
// 首次加载还没拿到数值时画骨架（文字条 + 空轨道呼吸），到达后原地替换；
// 已有数值的刷新不闪骨架，继续显示旧值。
ColumnLayout {
    id: quota

    property string used: ""
    property string limit: ""
    property bool loading: false
    /// < 0 表示不限量 / 未知：仍显示文字，但不画容量条
    property double ratio: -1

    /// 首次加载：请求在途且一个数值都还没拿到
    readonly property bool pending: quota.loading
                                     && quota.used.length === 0
                                     && quota.limit.length === 0
    readonly property bool ready: !quota.pending && quota.limit.length > 0
    readonly property bool showBar: quota.ready && quota.ratio >= 0
    readonly property string text: qsTr("已用 %1 / %2").arg(quota.used).arg(quota.limit)

    readonly property KTheme theme: KTheme { parent: quota }

    // 骨架和真实文字共用同一行高，容量数据到达时账号区不抽动
    FontMetrics {
        id: textMetrics
        font.pixelSize: quota.theme.fontCaption
    }
    readonly property real textHeight: textMetrics.height

    /// 骨架的呼吸透明度（只在首次加载时动）
    property real skeletonPulse: 1

    spacing: theme.gapXs

    ElidedLabel {
        objectName: "quotaText"
        visible: quota.ready
        Layout.fillWidth: true
        Layout.preferredHeight: quota.textHeight
        text: quota.text
        color: theme.textMuted
        font.pixelSize: theme.fontCaption
        elide: Text.ElideRight
        showToolTipWhenUntruncated: true
    }

    Rectangle {
        objectName: "quotaTrack"
        visible: quota.showBar
        Layout.fillWidth: true
        Layout.preferredHeight: 3
        radius: 1.5
        color: theme.surfacePressed

        Rectangle {
            objectName: "quotaFill"
            width: parent.width * Math.min(Math.max(quota.ratio, 0), 1)
            height: parent.height
            radius: parent.radius
            color: theme.accent
        }
    }

    // 首次加载的骨架：与真实内容同一竖排、同一高度，数据到达时只换内容不撑开侧栏
    ColumnLayout {
        objectName: "quotaSkeleton"
        visible: quota.pending
        Layout.fillWidth: true
        spacing: quota.spacing

        Item {
            Layout.fillWidth: true
            Layout.preferredHeight: quota.textHeight

            Rectangle {
                objectName: "quotaSkeletonText"
                width: 96
                height: 10
                anchors.verticalCenter: parent.verticalCenter
                radius: 2
                color: quota.theme.surfacePressed
                opacity: quota.skeletonPulse
            }
        }

        Rectangle {
            objectName: "quotaSkeletonTrack"
            Layout.fillWidth: true
            Layout.preferredHeight: 3
            radius: 1.5
            color: theme.surfacePressed
            opacity: quota.skeletonPulse
        }
    }

    SequentialAnimation {
        running: quota.pending
        loops: Animation.Infinite
        NumberAnimation {
            target: quota
            property: "skeletonPulse"
            from: 0.45
            to: 1.0
            duration: 750
            easing.type: Easing.InOutSine
        }
        NumberAnimation {
            target: quota
            property: "skeletonPulse"
            from: 1.0
            to: 0.45
            duration: 750
            easing.type: Easing.InOutSine
        }
    }
}
