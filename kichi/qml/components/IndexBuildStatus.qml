import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "../core"

// 底栏里的本地索引建库状态：不确定进度条 + 取消。
//
// 全盘列举官方不给总数，进度只能报已扫描条数（文案由 Rust 侧写进
// statusMessage），所以这里用不确定进度条。放在底栏而不是模态遮罩：
// 建库只是缓存预热，网络路径照常可用，锁住界面反而没有退路。
RowLayout {
    id: root

    property var backend: null

    readonly property bool building: backend !== null && backend.indexBuilding === true

    visible: building
    spacing: theme.gapSm

    KTheme { id: theme }

    KProgressBar {
        objectName: "indexBuildProgress"
        Layout.alignment: Qt.AlignVCenter
        Layout.preferredWidth: 96
        // 不在建库时停掉滚动动画，避免隐藏后还占一份 GUI 线程唤醒
        indeterminate: root.building
    }

    KButton {
        objectName: "indexBuildCancel"
        Layout.alignment: Qt.AlignVCenter
        text: qsTr("取消")
        variant: "ghost"
        implicitHeight: 24
        leftPadding: 8
        rightPadding: 8
        topPadding: 2
        bottomPadding: 2
        font.pixelSize: theme.fontMeta
        showTextToolTip: false
        onClicked: {
            if (root.backend)
                root.backend.cancelLocalIndexBuild()
        }
    }
}
