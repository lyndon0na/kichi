import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 全局底栏右侧的任务入口：本机传输 + 离线任务，空闲时安静显示。
Button {
    id: control

    property int activeCount: 0
    property int offlineCount: 0
    property bool pageActive: false

    signal requested()

    KTheme { id: theme }

    readonly property int totalCount: control.activeCount + control.offlineCount
    readonly property string countText: control.totalCount > 99
                                        ? "99+" : String(control.totalCount)
    readonly property string tooltipText: {
        if (control.totalCount === 0)
            return qsTr("打开任务")
        const parts = []
        if (control.activeCount > 0)
            parts.push(qsTr("本机传输 %1 个进行中").arg(control.activeCount))
        if (control.offlineCount > 0)
            parts.push(qsTr("离线任务 %1 条未结束").arg(control.offlineCount))
        return qsTr("任务 · %1").arg(parts.join(" · "))
    }

    text: qsTr("任务")
    implicitWidth: contentRow.implicitWidth + leftPadding + rightPadding
    implicitHeight: 24
    leftPadding: theme.gapSm
    rightPadding: theme.gapSm
    topPadding: 0
    bottomPadding: 0
    hoverEnabled: true
    focusPolicy: Qt.StrongFocus

    Accessible.name: control.text
    Accessible.description: control.totalCount > 0
                            ? qsTr("当前有 %1 个任务进行中或未结束").arg(control.totalCount)
                            : qsTr("打开任务")

    ToolTip.visible: control.hovered
    ToolTip.text: control.tooltipText

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
            color: control.pageActive || control.totalCount > 0
                   ? theme.accent : theme.textMuted
            opacity: control.pageActive || control.totalCount > 0 ? 1 : 0.8
        }

        Label {
            Layout.alignment: Qt.AlignVCenter
            text: control.text
            color: control.pageActive || control.totalCount > 0
                   ? theme.textPrimary : theme.textMuted
            font.pixelSize: theme.fontMeta
            font.weight: control.pageActive || control.totalCount > 0
                         ? Font.DemiBold : Font.Normal
        }

        KStatusBadge {
            objectName: "transferStatusCount"
            visible: control.totalCount > 0
            Layout.alignment: Qt.AlignVCenter
            text: control.countText
            tone: "accent"
        }
    }
}
