import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 文件页的列表 / 网格切换按钮。按钮显示当前视图，单击直接切到另一个视图；
// 搜索结果是独立列表，搜索期间禁用。
KButton {
    id: control

    property var backend: null

    readonly property string currentView: backend !== null
                                            && backend.fileView === "grid"
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
    enabled: backend !== null && backend.searchActive !== true
    showTextToolTip: false
    Accessible.name: qsTr("当前文件显示方式：%1").arg(text)
    ToolTip.visible: hovered
    ToolTip.text: !enabled
                  ? qsTr("搜索结果固定用列表显示")
                  : qsTr("点击切换到%1").arg(nextView === "grid"
                                              ? qsTr("网格视图")
                                              : qsTr("列表视图"))
    onClicked: viewRequested(nextView)
}
