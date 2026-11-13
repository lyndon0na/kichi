import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 底栏里的本地索引建库状态：不确定进度条。
//
// 全盘列举官方不给总数，进度只能报已扫描条数（文案由 Rust 侧写进
// statusMessage），所以这里用不确定进度条。放在底栏而不是模态遮罩：
// 建库期间界面照常可用。不提供停止：全盘搜索和建库共用同一次遍历，
// 中途停下只会浪费已经发出的请求；要中断只能关闭索引或退出应用。
RowLayout {
    id: root

    property var backend: null

    readonly property bool building: backend !== null && backend.indexBuilding === true

    visible: building
    KProgressBar {
        objectName: "indexBuildProgress"
        Layout.alignment: Qt.AlignVCenter
        Layout.preferredWidth: 96
        // 不在建库时停掉滚动动画，避免隐藏后还占一份 GUI 线程唤醒
        indeterminate: root.building
    }

}
