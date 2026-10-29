import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 传输页队列 / 历史区的紧凑行。正在下载的条目在上方摘要卡里，不进这个列表。
Item {
    id: row

    required property string key
    required property string name
    required property string sourceId
    required property string phase
    required property string stateText
    required property bool running
    required property int progress
    required property string progressText
    required property string sizeText
    required property string detail
    required property string path
    required property bool folder
    required property int childCount
    required property int failedCount
    required property string sectionKey

    signal openDetailsRequested(string key)

    property bool flash: false
    property var backend: null

    readonly property bool notice: phase === "notice"
    readonly property bool interrupting: phase === "interrupted"
    readonly property bool failed: phase === "failed"
    readonly property bool hasFailedChildren: folder && failedCount > 0
    readonly property bool canOpenDetails: folder && childCount > 0
    readonly property bool canReveal: path.length > 0
    readonly property bool canRetry: (failed || interrupting)
                                     && (!folder || childCount === 0)
    readonly property string detailText: {
        var text = detail
        if (text.length === 0)
            text = phase === "queued" ? qsTr("等待前序任务结束") : ""
        if (hasFailedChildren)
            text += qsTr("（点开可重试）")
        return text
    }

    implicitHeight: 60
    width: parent ? parent.width : 0

    KTheme { id: theme }

    HoverHandler {
        id: hover
    }

    function revealFile() {
        if (canReveal && backend !== null)
            backend.revealTransfer(key)
    }

    function retryDownload() {
        if (canRetry && backend !== null)
            backend.retryTransfer(key)
    }

    Rectangle {
        anchors.fill: parent
        radius: theme.radiusCard
        color: row.notice ? theme.surface
             : hover.hovered ? theme.surfaceHover
             : theme.surfaceRaised
        border.width: row.notice ? 0 : 1
        border.color: theme.hairline

        Rectangle {
            anchors.fill: parent
            radius: parent.radius
            color: theme.accentSoft
            border.width: 1
            border.color: theme.softTint(theme.accent, 0.42)
            opacity: row.flash ? 1 : 0

            Behavior on opacity {
                NumberAnimation { duration: 180 }
            }
        }
    }

    Label {
        visible: row.notice
        anchors.centerIn: parent
        text: row.name
        font.pixelSize: theme.fontMeta
        color: theme.textMuted
    }

    ColumnLayout {
        visible: !row.notice
        anchors.fill: parent
        anchors.leftMargin: 12
        anchors.rightMargin: 12
        anchors.topMargin: 8
        anchors.bottomMargin: 7
        spacing: 2

        RowLayout {
            Layout.fillWidth: true
            spacing: theme.gapSm

            Item {
                Layout.fillWidth: true
                Layout.minimumWidth: 0
                Layout.preferredHeight: 22

                RowLayout {
                    anchors.fill: parent
                    spacing: theme.gapSm

                    FileIcon {
                        kind: row.folder ? "folder" : ""
                        fileName: row.name
                        color: theme.textMuted
                        implicitWidth: 18
                        implicitHeight: 18
                    }

                    Label {
                        Layout.fillWidth: true
                        text: row.name
                        font.pixelSize: theme.fontBody
                        font.weight: row.folder ? Font.DemiBold : Font.Normal
                        color: theme.textPrimary
                        elide: Text.ElideMiddle
                        verticalAlignment: Text.AlignVCenter
                    }

                    Label {
                        objectName: "openDetailsHint"
                        visible: row.canOpenDetails
                        text: qsTr("查看 ›")
                        font.pixelSize: theme.fontCaption
                        color: theme.accent
                        verticalAlignment: Text.AlignVCenter
                    }
                }

                MouseArea {
                    id: rowHeader
                    objectName: "rowHeader"
                    anchors.fill: parent
                    enabled: row.canOpenDetails
                    hoverEnabled: enabled
                    cursorShape: enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
                    onClicked: row.openDetailsRequested(row.key)
                }
            }

            Label {
                objectName: "sizeText"
                text: row.sizeText
                font.pixelSize: theme.fontMeta
                color: theme.textMuted
                horizontalAlignment: Text.AlignRight
                Layout.preferredWidth: 120
                elide: Text.ElideLeft
            }

            KStatusBadge {
                objectName: "stateText"
                text: row.stateText
                tone: row.failed ? "danger"
                    : row.interrupting ? "warning"
                    : row.phase === "done" ? "success"
                    : row.phase === "running" ? "accent"
                    : "neutral"
            }
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.leftMargin: 26
            spacing: theme.gapSm

            Label {
                id: detailLabel

                objectName: "detailLink"
                Layout.fillWidth: true
                text: row.detailText
                font.pixelSize: theme.fontCaption
                color: row.failed || row.hasFailedChildren ? theme.danger
                     : row.interrupting ? theme.warning
                     : theme.textMuted
                elide: Text.ElideMiddle
                verticalAlignment: Text.AlignVCenter
                font.underline: detailMouse.containsMouse && row.canReveal && !row.folder

                MouseArea {
                    id: detailMouse
                    anchors.fill: parent
                    enabled: row.canReveal && !row.folder
                    hoverEnabled: true
                    cursorShape: enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
                    onClicked: row.revealFile()
                }
            }

            Label {
                objectName: "revealLink"
                visible: row.canReveal
                text: qsTr("打开位置")
                font.pixelSize: theme.fontCaption
                color: theme.accent
                font.underline: revealMouse.containsMouse

                MouseArea {
                    id: revealMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: row.revealFile()
                }
            }

            Label {
                objectName: "retryLink"
                visible: row.canRetry
                text: qsTr("重试")
                font.pixelSize: theme.fontCaption
                color: theme.accent
                font.underline: retryMouse.containsMouse

                ToolTip.visible: row.interrupting && retryMouse.containsMouse
                ToolTip.delay: 400
                ToolTip.text: qsTr("上次未完成，若下载目录未变会从断点续传")

                MouseArea {
                    id: retryMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: row.retryDownload()
                }
            }
        }
    }
}
