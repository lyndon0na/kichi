import QtQuick
import QtQuick.Controls

// 一行圆形色板：点选颜色，选中项加圈 + 对勾。
//
// `colors` 每项：{ color: "#AARRGGBB", name: "白色", darkText: true }。
// `darkText` 只决定对勾用黑还是白（浅色块上黑勾才看得见）。
Item {
    id: row

    property var colors: []
    property string current: ""
    /// 选中圈的描边色：深色浮层用白色，设置页传强调色
    property color selectionColor: "white"

    signal colorPicked(string color)

    implicitWidth: swatch.implicitWidth
    implicitHeight: 22

    Row {
        id: swatch

        spacing: 8

        Repeater {
            model: row.colors

            delegate: Rectangle {
                required property var modelData

                readonly property bool selected: row.current.toUpperCase()
                                                 === modelData.color.toUpperCase()

                width: 22
                height: 22
                radius: 11
                color: modelData.color
                border.width: selected ? 2 : 1
                border.color: selected ? row.selectionColor : "#44FFFFFF"

                NavIcon {
                    anchors.centerIn: parent
                    width: 12
                    height: 12
                    name: "check"
                    visible: parent.selected
                    color: modelData.darkText ? "#FF000000" : "#FFFFFFFF"
                }

                MouseArea {
                    anchors.fill: parent
                    onClicked: row.colorPicked(modelData.color)
                }

                ToolTip.visible: hover.hovered
                ToolTip.text: modelData.name
                HoverHandler { id: hover }
            }
        }
    }
}
