import QtQuick
import QtQuick.Controls
import "../components"

// 文件类内容页共用的列表 / 网格切换按钮。
// 按钮只发出目标视图，页面负责先记住当前滚动位置，再切换全局设置。
KButton {
    id: control

    property var backend: null

    readonly property string currentView: backend !== null
                                            && backend.contentView === "grid"
                                          ? "grid" : "list"
    readonly property string nextView: currentView === "grid" ? "list" : "grid"

    signal viewRequested(string view)

    objectName: "viewToggleButton"
    variant: "ghost"
    iconName: currentView === "grid" ? "layout-grid" : "list"
    iconSize: 15
    text: currentView === "grid" ? qsTr("网格") : qsTr("列表")
    implicitHeight: 34
    leftPadding: 8
    rightPadding: 8
    enabled: backend !== null
    showTextToolTip: false
    Accessible.name: qsTr("当前内容显示方式：%1").arg(text)
    ToolTip.visible: hovered
    ToolTip.text: qsTr("点击切换到%1").arg(nextView === "grid"
                                            ? qsTr("网格视图")
                                            : qsTr("列表视图"))
    onClicked: viewRequested(nextView)
}
