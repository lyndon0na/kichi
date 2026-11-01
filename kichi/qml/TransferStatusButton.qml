import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 全局底栏右侧的传输入口：空闲时安静显示，有任务时用数量标出进度。
Button {
    id: control

    property int activeCount: 0
    property bool pageActive: false

    signal requested()

    KTheme { id: theme }

    readonly property string countText: control.activeCount > 99
                                        ? "99+" : String(control.activeCount)

    text: qsTr("传输任务")
    implicitWidth: contentRow.implicitWidth + leftPadding + rightPadding
    implicitHeight: 24
    leftPadding: theme.gapSm
    rightPadding: theme.gapSm
    topPadding: 0
    bottomPadding: 0
    hoverEnabled: true
    focusPolicy: Qt.StrongFocus

    Accessible.name: control.text
    Accessible.description: control.activeCount > 0
                            ? qsTr("%1 个传输进行中").arg(control.activeCount)
                            : qsTr("打开传输任务")

    ToolTip.visible: control.hovered
    ToolTip.text: control.activeCount > 0
                  ? qsTr("传输任务 · %1 个进行中").arg(control.activeCount)
                  : qsTr("打开传输任务")

    onClicked: control.requested()

    background: Rectangle {
        radius: theme.radiusSmall
        color: control.down ? theme.surfacePressed
             : control.pageActive ? theme.accentGhost
             : control.hovered ? theme.surfaceHover
             : theme.surfaceClear
    }

    contentItem: RowLayout {
        id: contentRow

        spacing: 6

        NavIcon {
            implicitWidth: 14
            implicitHeight: 14
            Layout.alignment: Qt.AlignVCenter
            name: "transfer"
            color: control.pageActive || control.activeCount > 0
                   ? theme.accent : theme.textMuted
            opacity: control.pageActive || control.activeCount > 0 ? 1 : 0.8
        }

        Label {
            Layout.alignment: Qt.AlignVCenter
            text: control.text
            color: control.pageActive || control.activeCount > 0
                   ? theme.textPrimary : theme.textMuted
            font.pixelSize: theme.fontMeta
            font.weight: control.pageActive || control.activeCount > 0
                         ? Font.DemiBold : Font.Normal
        }

        KStatusBadge {
            objectName: "transferStatusCount"
            visible: control.activeCount > 0
            Layout.alignment: Qt.AlignVCenter
            text: control.countText
            tone: "accent"
        }
    }
}
