import QtQuick
import QtQuick.Layouts
import "../components"
import "../core"

// 底栏右侧的容量指示：已用 / 上限文字 + 3px 容量条。
//
// 从账号区整块挪来（2026-11-05）：账号行只留头像、名字和设置入口，
// 容量这种长期不变的数字放到不抢视线、又始终可见的底栏。
// 首次加载还没拿到数值时画骨架（文字条 + 空轨道呼吸），到达后原地替换；
// 已有数值的刷新不闪骨架，继续显示旧值。
RowLayout {
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

    /// 骨架的呼吸透明度（只在首次加载时动）
    property real skeletonPulse: 1

    spacing: theme.gapSm

    ElidedLabel {
        objectName: "quotaText"
        visible: quota.ready
        Layout.alignment: Qt.AlignVCenter
        Layout.maximumWidth: 200
        text: quota.text
        color: theme.textMuted
        font.pixelSize: theme.fontMeta
        elide: Text.ElideRight
        showToolTipWhenUntruncated: true
    }

    Rectangle {
        objectName: "quotaTrack"
        visible: quota.showBar
        Layout.alignment: Qt.AlignVCenter
        Layout.preferredWidth: 56
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

    // 首次加载的骨架：与真实内容同一横排、同一尺寸，数据到达时只换内容不撑开底栏
    RowLayout {
        objectName: "quotaSkeleton"
        visible: quota.pending
        Layout.alignment: Qt.AlignVCenter
        spacing: quota.spacing

        Rectangle {
            objectName: "quotaSkeletonText"
            Layout.preferredWidth: 96
            Layout.preferredHeight: 10
            radius: 2
            color: theme.surfacePressed
            opacity: quota.skeletonPulse
        }

        Rectangle {
            objectName: "quotaSkeletonTrack"
            Layout.preferredWidth: 56
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
