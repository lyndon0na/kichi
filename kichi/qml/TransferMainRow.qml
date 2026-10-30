import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 传输任务单列表的一行：父批次与文件夹子文件共用同一套列。
//
// 固定 48px 单行；失败原因并入状态列，状态说明过长时放悬停提示。
// 文件夹父行只负责展开 / 收起，子行通过 depth=1 画层级线并缩进图标。
ItemDelegate {
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
    required property int depth
    required property string rowKind
    required property string parentKey
    required property bool expandable
    required property bool expanded

    property bool flash: false
    property bool sticky: false
    property bool showSize: true
    property bool showRate: true
    property var live: ({})

    signal toggleRequested(string key)
    signal pauseRequested(string key)
    signal resumeRequested(string key)
    signal cancelRequested(string key)
    signal retryRequested(string key)
    signal revealRequested(string key)

    objectName: "transferRow_" + row.key
    implicitHeight: density.rowHeight
    leftPadding: 0
    rightPadding: 0
    topPadding: 0
    bottomPadding: 0

    readonly property bool child: row.rowKind === "child"
    readonly property bool notice: row.phase === "notice"
    readonly property bool failed: row.phase === "failed"
    readonly property bool interrupted: row.phase === "interrupted"
    readonly property bool pausing: row.phase === "pausing"
    readonly property bool paused: row.phase === "paused"
    readonly property bool active: pagePhaseActive(row.phase)
    readonly property bool showProgress: row.active || row.phase === "done"
                                         || row.pausing || row.paused
                                         || row.currentProgress > 0
                                         || row.currentProgress < 0
    readonly property bool canPause: row.active && !row.pausing
                                     && !row.child && !row.notice
    readonly property bool canResume: row.paused && !row.child && !row.notice
    readonly property bool canCancel: !row.notice
                                      && ((row.pausing || row.paused)
                                          ? !row.child : row.active)
    readonly property bool canRetry: (row.failed || row.interrupted)
                                     && (!row.folder || row.childCount === 0)
    readonly property bool canReveal: row.path.length > 0
    readonly property int currentProgress: row.liveProgress(row.progress)
    readonly property string currentProgressText: row.progressLabel()
    readonly property string currentSizeText: row.liveField("sizeText", row.sizeText)
    readonly property string currentRateText: row.rateLabel()
    readonly property string displayStatus: row.statusLabel()
    readonly property string tone: row.failed ? "danger"
        : row.interrupted ? "warning"
        : row.pausing || row.paused ? "warning"
        : row.phase === "done" ? "success"
        : row.active ? "accent"
        : "neutral"

    TransferTableDensity { id: density }
    KTheme { id: theme }

    HoverHandler {
        id: rowHover
    }

    function pagePhaseActive(phase) {
        return phase === "queued" || phase === "preparing"
                || phase === "running" || phase === "pausing"
    }

    function liveField(field, fallback) {
        if (row.live === null || row.live === undefined)
            return fallback
        var entry = row.live[row.key]
        if (entry === undefined || entry === null)
            return fallback
        var value = entry[field]
        return value === undefined ? fallback : value
    }

    function liveProgress(fallback) {
        return row.liveField("progress", fallback)
    }

    function progressLabel() {
        var value = row.currentProgress
        var label = row.liveField("progressText", row.progressText)
        if (label.length > 0)
            return label
        if (row.phase === "done")
            return "100%"
        return value >= 0 ? value + "%" : ""
    }

    function rateLabel() {
        var rate = row.liveField("rateText", "")
        var note = row.liveField("noteText", "")
        if (note.length === 0)
            return rate
        return rate.length === 0 ? note : rate + " · " + note
    }

    function statusLabel() {
        if (row.failed) {
            if (row.folder && row.childCount > 0 && row.failedCount > 0)
                return row.stateText + " · " + row.failedCount + qsTr(" 个失败")
            if (row.detail.length > 0)
                return row.stateText + " · " + row.detail
        }
        if (row.interrupted && row.detail.length > 0)
            return row.stateText + " · " + row.detail
        if (row.phase === "cancelled" && row.detail.length > 0)
            return row.stateText + " · " + row.detail
        var skipAt = row.detail.indexOf(qsTr("跳过"))
        if (row.phase === "done" && skipAt >= 0)
            return row.stateText + " · " + row.detail.substring(skipAt)
        return row.stateText
    }

    function statusTooltip() {
        if (row.detail.length === 0 || row.detail === row.displayStatus)
            return row.displayStatus
        return row.displayStatus + "\n" + row.detail
    }

    background: Rectangle {
        radius: 0
        color: row.notice ? theme.surface
             : row.active || row.expanded
               ? (row.sticky ? Qt.tint(theme.surface, theme.accentGhost)
                             : theme.accentGhost)
             : rowHover.hovered ? theme.surfaceHover
             : theme.surfaceClear

        Rectangle {
            visible: row.active && !row.child
            width: 3
            height: parent.height - 14
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
            color: row.sticky ? theme.hairline : theme.divider
        }

        Rectangle {
            anchors.fill: parent
            color: theme.accentSoft
            opacity: row.flash ? 1 : 0

            Behavior on opacity {
                NumberAnimation { duration: 180 }
            }
        }
    }

    contentItem: RowLayout {
        anchors.fill: parent
        anchors.leftMargin: density.rowPadding
        anchors.rightMargin: density.rowPadding
        spacing: density.columnSpacing

        Item {
            Layout.preferredWidth: density.iconColumnWidth
                                    + row.depth * density.indentWidth
            Layout.minimumWidth: density.iconColumnWidth
                                  + row.depth * density.indentWidth
            Layout.fillHeight: true

            Rectangle {
                visible: row.child
                x: row.depth * density.indentWidth - 1
                y: 0
                width: 1
                height: parent.height
                color: theme.divider
            }

            Rectangle {
                visible: row.child
                x: row.depth * density.indentWidth - 1
                y: parent.height / 2
                width: density.indentWidth - 7
                height: 1
                color: theme.divider
            }

            NavIcon {
                id: expandArrow
                objectName: "openDetailsArrow"
                visible: row.folder && row.expandable
                width: 12
                height: 12
                x: 1
                anchors.verticalCenter: parent.verticalCenter
                name: "chevron-right"
                color: theme.textMuted
                rotation: row.expanded ? 90 : 0

                Behavior on rotation {
                    NumberAnimation { duration: 120 }
                }

                MouseArea {
                    objectName: "openDetailsArrowHit"
                    anchors.fill: parent
                    anchors.margins: -4
                    cursorShape: Qt.PointingHandCursor
                    onClicked: row.toggleRequested(row.key)
                }
            }

            FileIcon {
                objectName: "rowTypeIcon"
                width: 18
                height: 18
                // 顶层普通文件也空出箭头槽，和文件夹图标落在同一竖线；
                // 子文件保持现有缩进，不再额外右移。
                x: row.child ? row.depth * density.indentWidth + 10 : 18
                anchors.verticalCenter: parent.verticalCenter
                kind: row.folder ? "folder" : ""
                fileName: row.name
                color: row.folder ? theme.accent : theme.textSecondary
            }
        }

        Item {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            Layout.fillHeight: true

            Label {
                objectName: "nameText"
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                text: row.name
                color: row.notice ? theme.textMuted : theme.textPrimary
                font.pixelSize: theme.fontBody
                font.weight: row.folder ? Font.DemiBold : Font.Normal
                elide: Text.ElideMiddle
                verticalAlignment: Text.AlignVCenter
            }

            MouseArea {
                objectName: "rowHeader"
                anchors.fill: parent
                enabled: row.expandable
                hoverEnabled: enabled
                cursorShape: enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
                onClicked: row.toggleRequested(row.key)
            }
        }

        Item {
            Layout.preferredWidth: density.statusColumnWidth
            Layout.minimumWidth: density.statusColumnWidth
            Layout.fillHeight: true

            HoverHandler {
                id: statusHover
            }

            Rectangle {
                width: density.statusColumnWidth - 8
                height: 22
                anchors.centerIn: parent
                radius: theme.radiusSmall
                color: theme.softTint(statusToneColor(), row.tone === "neutral" ? 0.10 : 0.14)

                Label {
                    id: statusLabel
                    objectName: "stateText"
                    anchors.fill: parent
                    anchors.leftMargin: 7
                    anchors.rightMargin: 7
                    text: row.displayStatus
                    color: statusToneColor()
                    font.pixelSize: theme.fontCaption
                    font.weight: Font.DemiBold
                    horizontalAlignment: Text.AlignHCenter
                    verticalAlignment: Text.AlignVCenter
                    elide: Text.ElideRight
                }
            }

            ToolTip.visible: statusHover.hovered && row.statusTooltip().length > 0
            ToolTip.delay: 500
            ToolTip.text: row.statusTooltip()
        }

        Item {
            Layout.preferredWidth: density.progressColumnWidth
            Layout.minimumWidth: density.progressColumnWidth
            Layout.fillHeight: true

            RowLayout {
                anchors.fill: parent
                visible: row.showProgress && !row.notice
                spacing: theme.gapSm

                KProgressBar {
                    objectName: "rowProgress"
                    Layout.fillWidth: true
                    Layout.minimumWidth: 50
                    Layout.alignment: Qt.AlignVCenter
                    from: 0
                    to: 100
                    value: Math.max(row.currentProgress, 0)
                    indeterminate: row.currentProgress < 0
                }

                Label {
                    objectName: "progressText"
                    Layout.preferredWidth: 42
                    text: row.currentProgressText
                    color: theme.textMuted
                    font.pixelSize: theme.fontMeta
                    horizontalAlignment: Text.AlignRight
                    verticalAlignment: Text.AlignVCenter
                    elide: Text.ElideRight
                }
            }

            Label {
                anchors.centerIn: parent
                visible: !row.showProgress || row.notice
                text: "—"
                color: theme.textMuted
                font.pixelSize: theme.fontMeta
            }
        }

        Label {
            objectName: "sizeText"
            visible: row.showSize
            Layout.preferredWidth: row.showSize ? density.sizeColumnWidth : 0
            Layout.minimumWidth: row.showSize ? density.sizeColumnWidth : 0
            text: row.currentSizeText
            color: theme.textMuted
            font.pixelSize: theme.fontMeta
            horizontalAlignment: Text.AlignRight
            verticalAlignment: Text.AlignVCenter
            elide: Text.ElideLeft
        }

        Label {
            objectName: "rateText"
            visible: row.showRate
            Layout.preferredWidth: row.showRate ? density.rateColumnWidth : 0
            Layout.minimumWidth: row.showRate ? density.rateColumnWidth : 0
            text: row.running && row.currentRateText.length > 0
                  ? row.currentRateText : "—"
            color: theme.textMuted
            font.pixelSize: theme.fontMeta
            horizontalAlignment: Text.AlignRight
            verticalAlignment: Text.AlignVCenter
            elide: Text.ElideRight
        }

        Item {
            Layout.preferredWidth: density.actionColumnWidth
            Layout.minimumWidth: density.actionColumnWidth
            Layout.fillHeight: true

            RowLayout {
                anchors.centerIn: parent
                spacing: 2

                KButton {
                    objectName: "pauseLink"
                    visible: row.canPause
                    text: qsTr("暂停")
                    variant: "ghost"
                    implicitHeight: 26
                    leftPadding: 7
                    rightPadding: 7
                    topPadding: 2
                    bottomPadding: 2
                    font.pixelSize: theme.fontMeta
                    onClicked: row.pauseRequested(row.key)
                }

                KButton {
                    objectName: "resumeLink"
                    visible: row.canResume
                    text: qsTr("继续")
                    variant: "ghost"
                    implicitHeight: 26
                    leftPadding: 7
                    rightPadding: 7
                    topPadding: 2
                    bottomPadding: 2
                    font.pixelSize: theme.fontMeta
                    onClicked: row.resumeRequested(row.key)
                }

                KButton {
                    objectName: "cancelLink"
                    visible: row.canCancel
                    text: qsTr("取消")
                    variant: "ghost"
                    implicitHeight: 26
                    leftPadding: 7
                    rightPadding: 7
                    topPadding: 2
                    bottomPadding: 2
                    font.pixelSize: theme.fontMeta
                    onClicked: row.cancelRequested(row.key)
                }

                KButton {
                    objectName: "retryLink"
                    visible: row.canRetry
                    text: qsTr("重试")
                    variant: "ghost"
                    implicitHeight: 26
                    leftPadding: 7
                    rightPadding: 7
                    topPadding: 2
                    bottomPadding: 2
                    font.pixelSize: theme.fontMeta
                    ToolTip.visible: row.interrupted && hovered
                    ToolTip.delay: 400
                    ToolTip.text: qsTr("上次未完成，若下载目录未变会从断点续传")
                    onClicked: row.retryRequested(row.key)
                }

                KButton {
                    objectName: "revealLink"
                    visible: row.canReveal
                    text: qsTr("打开位置")
                    variant: "ghost"
                    implicitHeight: 26
                    leftPadding: 7
                    rightPadding: 7
                    topPadding: 2
                    bottomPadding: 2
                    font.pixelSize: theme.fontMeta
                    onClicked: row.revealRequested(row.key)
                }
            }
        }
    }

    function statusToneColor() {
        if (row.tone === "success")
            return theme.success
        if (row.tone === "warning")
            return theme.warning
        if (row.tone === "danger")
            return theme.danger
        if (row.tone === "accent")
            return theme.accent
        return theme.textMuted
    }
}
