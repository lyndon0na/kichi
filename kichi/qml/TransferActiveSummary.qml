import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 传输页顶部摘要：独立文件共享一张「不同来源」卡片，文件夹批次用文件夹名作标题。
// 分片只是同一文件内部的连接，不在这里拆成多行。
Item {
    id: summary

    property var group: null
    property string highlightedKey: ""

    signal openDetailsRequested(string key)

    readonly property bool hasGroup: group !== null && group !== undefined
    readonly property bool preparing: hasGroup && group.state === "preparing"
    readonly property bool folder: hasGroup && group.kind === "folder"
    readonly property var lanes: hasGroup && group.lanes !== undefined ? group.lanes : []
    readonly property int runningCount: hasGroup && group.runningCount !== undefined
                                        ? group.runningCount : 0
    readonly property int hiddenCount: hasGroup && group.hiddenCount !== undefined
                                       ? group.hiddenCount : 0
    readonly property bool flashed: hasGroup && highlightedKey === group.groupKey
    readonly property bool canOpenDetails: folder && group.totalCount > 0

    visible: hasGroup
    implicitHeight: visible ? card.implicitHeight : 0

    function titleText() {
        if (!hasGroup)
            return ""
        if (folder) {
            return preparing
                ? qsTr("正在准备「%1」").arg(group.name)
                : group.name
        }
        return qsTr("正在下载 %1 个文件").arg(runningCount)
    }

    function subtitleText() {
        if (!hasGroup)
            return ""
        if (preparing)
            return qsTr("正在读取目录结构…")
        if (folder)
            return qsTr("%1 个文件同时下载").arg(runningCount)
        return qsTr("不同来源")
    }

    function badgeText() {
        if (!hasGroup)
            return ""
        if (preparing)
            return qsTr("准备中")
        return qsTr("%1 个并发").arg(runningCount)
    }

    KTheme { id: theme }

    Rectangle {
        id: card

        width: parent.width
        implicitHeight: content.implicitHeight + 28
        radius: theme.radiusSurface
        color: theme.surfaceRaised
        border.width: summary.flashed ? 2 : 1
        border.color: summary.flashed
                      ? theme.softTint(theme.accent, 0.72)
                      : theme.softTint(theme.accent, 0.28)
        clip: true

        Behavior on border.color {
            ColorAnimation { duration: 180 }
        }

        // 左侧一根极窄强调线：让摘要和下面的普通历史卡片拉开层级。
        Rectangle {
            anchors.left: parent.left
            anchors.top: parent.top
            anchors.bottom: parent.bottom
            width: 3
            color: theme.accent
            opacity: 0.85
        }

        ColumnLayout {
            id: content

            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.leftMargin: 16
            anchors.rightMargin: 16
            anchors.topMargin: 14
            spacing: 10

            RowLayout {
                Layout.fillWidth: true
                spacing: theme.gapSm

                NavIcon {
                    name: summary.folder ? "folder" : "transfer"
                    color: theme.accent
                    implicitWidth: 18
                    implicitHeight: 18
                }

                Label {
                    id: titleLabel

                    objectName: "activeSummaryTitle"
                    Layout.fillWidth: true
                    text: summary.titleText()
                    font.pixelSize: theme.fontSection
                    font.weight: Font.DemiBold
                    color: theme.textPrimary
                    elide: Text.ElideMiddle
                    font.underline: titleMouse.containsMouse && summary.canOpenDetails

                    MouseArea {
                        id: titleMouse
                        anchors.fill: parent
                        enabled: summary.canOpenDetails
                        hoverEnabled: enabled
                        cursorShape: enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
                        onClicked: summary.openDetailsRequested(summary.group.groupKey)
                    }
                }

                Label {
                    objectName: "activeSummarySubtitle"
                    text: summary.subtitleText()
                    font.pixelSize: theme.fontMeta
                    color: theme.textMuted
                    elide: Text.ElideRight
                    Layout.maximumWidth: 240
                }

                Label {
                    text: summary.hasGroup ? summary.group.sizeText : ""
                    font.pixelSize: theme.fontMeta
                    color: theme.textSecondary
                    horizontalAlignment: Text.AlignRight
                    Layout.preferredWidth: 150
                    elide: Text.ElideLeft
                }

                KStatusBadge {
                    objectName: "activeSummaryBadge"
                    text: summary.badgeText()
                    tone: summary.preparing ? "warning" : "accent"
                }
            }

            GridLayout {
                Layout.fillWidth: true
                Layout.leftMargin: 26
                columns: Math.max(1, Math.min(summary.lanes.length,
                                              Math.floor(summary.width / 300)))
                columnSpacing: 18
                rowSpacing: 12
                visible: !summary.preparing

                Repeater {
                    model: summary.lanes

                    delegate: ColumnLayout {
                        id: lane

                        required property var modelData

                        readonly property bool flashed: summary.highlightedKey === modelData.key
                        readonly property string note: modelData.noteText || ""

                        objectName: "activeLane-" + modelData.key
                        Layout.fillWidth: true
                        Layout.minimumWidth: 0
                        Layout.preferredWidth: 220
                        spacing: 3

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: theme.gapSm

                            FileIcon {
                                fileName: lane.modelData.name
                                kind: ""
                                color: theme.textMuted
                                implicitWidth: 16
                                implicitHeight: 16
                            }

                            Label {
                                Layout.fillWidth: true
                                text: lane.modelData.name
                                font.pixelSize: theme.fontMeta
                                font.weight: lane.flashed ? Font.DemiBold : Font.Normal
                                color: lane.flashed ? theme.accent : theme.textPrimary
                                elide: Text.ElideMiddle
                            }

                            Rectangle {
                                id: shardTag

                                readonly property string shardText: lane.modelData.shardText || ""

                                visible: shardText.length > 0
                                implicitWidth: shardLabel.implicitWidth + 10
                                implicitHeight: 18
                                radius: theme.radiusSmall
                                color: theme.accentGhost

                                Label {
                                    id: shardLabel
                                    anchors.centerIn: parent
                                    text: shardTag.shardText
                                    font.pixelSize: theme.fontCaption
                                    color: theme.accent
                                }
                            }

                            Label {
                                objectName: "activeLaneSize"
                                text: lane.modelData.sizeText || ""
                                font.pixelSize: theme.fontCaption
                                color: theme.textMuted
                                horizontalAlignment: Text.AlignRight
                                Layout.preferredWidth: 104
                                Layout.minimumWidth: 72
                                elide: Text.ElideLeft
                            }
                        }

                        RowLayout {
                            Layout.fillWidth: true
                            Layout.leftMargin: 24
                            spacing: theme.gapSm

                            KProgressBar {
                                objectName: "activeLaneProgress"
                                Layout.fillWidth: true
                                Layout.minimumWidth: 80
                                from: 0
                                to: 100
                                value: Math.max(lane.modelData.progress, 0)
                                indeterminate: lane.modelData.progress < 0
                            }

                            Label {
                                objectName: "activeLaneProgressText"
                                text: lane.modelData.progressText || ""
                                font.pixelSize: theme.fontCaption
                                color: theme.textSecondary
                                horizontalAlignment: Text.AlignRight
                                Layout.preferredWidth: 42
                            }

                            Label {
                                objectName: "activeLaneRate"
                                Layout.preferredWidth: 116
                                Layout.minimumWidth: 80
                                text: {
                                    var base = lane.modelData.rateText || ""
                                    if (lane.note.length === 0)
                                        return base
                                    return base.length === 0
                                        ? lane.note
                                        : base + " · " + lane.note
                                }
                                font.pixelSize: theme.fontCaption
                                color: theme.textMuted
                                horizontalAlignment: Text.AlignRight
                                elide: Text.ElideRight
                            }
                        }
                    }
                }
            }

            Label {
                visible: summary.hiddenCount > 0
                Layout.leftMargin: 26
                text: qsTr("另有 %1 个文件正在下载").arg(summary.hiddenCount)
                font.pixelSize: theme.fontCaption
                color: theme.textMuted
            }
        }
    }
}
