import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 紧凑步进器：「− 数值 +」。
//
// 自身不存值：`value` 由调用方绑定，点击只发 `valueEdited`，
// 由调用方写回后端再通过属性绑定刷新。这样后端更新（设置页改动 /
// mpv 原生键位）不会被内部控制赋值打断绑定。
Item {
    id: stepper

    property int value: 0
    property int from: 0
    property int to: 100
    property int stepSize: 1
    property string suffix: ""
    property color foreground: "white"
    /// 读屏用的控件名（例如「字幕字号」）
    property string accessibleName: ""

    signal valueEdited(int value)

    implicitWidth: 108
    implicitHeight: 28

    function step(direction) {
        var next = Math.max(stepper.from,
                            Math.min(stepper.to, stepper.value + direction * stepper.stepSize))
        if (next !== stepper.value)
            stepper.valueEdited(next)
    }

    RowLayout {
        anchors.fill: parent
        spacing: 0

        KIconButton {
            objectName: "stepperMinus"
            iconName: "minus"
            iconSize: 14
            iconColor: stepper.foreground
            iconOpacity: 0.85
            implicitWidth: 26
            implicitHeight: 26
            focusPolicy: Qt.NoFocus
            enabled: stepper.enabled && stepper.value > stepper.from
            Accessible.name: qsTr("减小")
            onClicked: stepper.step(-1)
        }

        Label {
            Layout.fillWidth: true
            horizontalAlignment: Text.AlignHCenter
            text: stepper.value + stepper.suffix
            color: stepper.foreground
            font.pixelSize: 12
            Accessible.name: stepper.accessibleName
        }

        KIconButton {
            objectName: "stepperPlus"
            iconName: "plus"
            iconSize: 14
            iconColor: stepper.foreground
            iconOpacity: 0.85
            implicitWidth: 26
            implicitHeight: 26
            focusPolicy: Qt.NoFocus
            enabled: stepper.enabled && stepper.value < stepper.to
            Accessible.name: qsTr("增大")
            onClicked: stepper.step(1)
        }
    }
}
