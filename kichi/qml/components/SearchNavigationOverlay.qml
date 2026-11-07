import QtQuick

// 搜索结果跳目录时先保留当前结果，等目标目录读回来再换 model。
// 加载状态由面包屑的转圈表示，这层只负责挡住重复操作。
Item {
    id: root

    objectName: "searchNavigationOverlay"
    property bool running: false

    visible: running
    z: 10

    MouseArea {
        anchors.fill: parent
        acceptedButtons: Qt.AllButtons
        hoverEnabled: true

        onWheel: function (wheel) {
            wheel.accepted = true
        }
    }
}
