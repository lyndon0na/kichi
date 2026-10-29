import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 离线任务表格的一行。状态、进度和删除入口固定列对齐，
// 大小 / 时间或失败原因收在任务名下方，避免卡片高度参差不齐。
ItemDelegate {
    id: row

    required property string key
    required property string name
    required property string phase
    required property string phaseText
    required property int progress
    required property string progressText
    required property string sizeText
    required property string createdText
    required property string detail
    required property bool settled

    property int rowHeight: 56
    property int statusColumnWidth: 88
    property int progressColumnWidth: 150
    property int actionColumnWidth: 64
    property int columnSpacing: 12
    property int contentPadding: 12

    signal deleteRequested(string key, string name)

    objectName: "taskRow_" + row.key
    implicitHeight: row.rowHeight
    leftPadding: 0
    rightPadding: 0
    topPadding: 0
    bottomPadding: 0

    KTheme { id: theme }

    readonly property bool isActive: !row.settled
    readonly property bool showProgress: row.phase === "pending"
                                              || row.phase === "running"
                                              || row.phase === "paused"

    readonly property string tone: {
        if (row.phase === "error")
            return "danger"
        if (row.phase === "completed")
            return "success"
        if (row.phase === "pending" || row.phase === "running")
            return "accent"
        return "neutral"
    }

    readonly property string metaText: {
        if (row.phase === "error" && row.detail.length > 0)
            return row.detail
        var parts = []
        if (row.sizeText.length > 0)
            parts.push(row.sizeText)
        if (row.createdText.length > 0)
            parts.push(row.createdText)
        return parts.join(" · ")
    }

    HoverHandler {
        id: rowHover
    }

    background: Rectangle {
        radius: theme.radiusSmall
        color: row.isActive
               ? theme.accentGhost
               : rowHover.hovered
                 ? theme.surfaceHover
                 : theme.surfaceClear

        Rectangle {
            visible: row.isActive
            width: 3
            height: parent.height - 20
            radius: 1.5
            anchors.left: parent.left
            anchors.leftMargin: 1
            anchors.verticalCenter: parent.verticalCenter
            color: theme.accent
        }

        Rectangle {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: 1
            color: theme.divider
        }
    }

    contentItem: RowLayout {
        anchors.fill: parent
        anchors.leftMargin: row.contentPadding
        anchors.rightMargin: row.contentPadding
        spacing: row.columnSpacing

        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true

            HoverHandler {
                id: nameHover
            }

            Column {
                width: parent.width
                anchors.verticalCenter: parent.verticalCenter
                spacing: 2

                Label {
                    width: parent.width
                    text: row.name
                    color: theme.textPrimary
                    font.pixelSize: theme.fontSection
                    font.weight: Font.DemiBold
                    elide: Text.ElideMiddle
                }

                Label {
                    width: parent.width
                    visible: text.length > 0
                    text: row.metaText
                    color: row.phase === "error" ? theme.danger : theme.textMuted
                    font.pixelSize: theme.fontMeta
                    elide: Text.ElideRight
                }
            }

            ToolTip {
                visible: nameHover.hovered
                         && row.phase === "error"
                         && row.detail.length > 0
                text: row.detail
                delay: 700
            }
        }

        Item {
            Layout.preferredWidth: row.statusColumnWidth
            Layout.minimumWidth: row.statusColumnWidth
            Layout.fillHeight: true

            KStatusBadge {
                anchors.centerIn: parent
                text: row.phaseText
                tone: row.tone
            }
        }

        Item {
            Layout.preferredWidth: row.progressColumnWidth
            Layout.minimumWidth: row.progressColumnWidth
            Layout.fillHeight: true

            RowLayout {
                anchors.fill: parent
                visible: row.showProgress
                spacing: theme.gapSm

                KProgressBar {
                    Layout.fillWidth: true
                    Layout.alignment: Qt.AlignVCenter
                    from: 0
                    to: 100
                    value: row.progress
                    indeterminate: row.phase === "pending"
                }

                Label {
                    Layout.preferredWidth: 34
                    text: row.progressText
                    color: theme.textMuted
                    font.pixelSize: theme.fontMeta
                    horizontalAlignment: Text.AlignRight
                    verticalAlignment: Text.AlignVCenter
                }
            }

            Label {
                anchors.centerIn: parent
                visible: !row.showProgress
                text: "—"
                color: theme.textMuted
                font.pixelSize: theme.fontMeta
            }
        }

        Item {
            Layout.preferredWidth: row.actionColumnWidth
            Layout.minimumWidth: row.actionColumnWidth
            Layout.fillHeight: true

            KButton {
                anchors.centerIn: parent
                implicitHeight: 28
                leftPadding: 8
                rightPadding: 8
                text: qsTr("删除")
                variant: "ghost"
                Accessible.name: qsTr("删除离线任务「%1」").arg(row.name)
                onClicked: row.deleteRequested(row.key, row.name)
            }
        }
    }
}
