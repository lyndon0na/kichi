import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 离线任务表格的一行。双击已完成产物：文件夹进文件页，视频 / 音频走播放；
// 删除不再占一列，统一放到右键菜单。
ItemDelegate {
    id: row

    required property string key
    required property string name
    required property string kind
    required property string phase
    required property string phaseText
    required property int progress
    required property string progressText
    required property string sizeText
    required property string createdText
    required property string detail
    required property bool settled
    required property string fileId
    required property bool hasThumbnail
    required property bool video
    required property bool playable

    property var backend: null
    property int rowHeight: 48
    property int thumbnailColumnWidth: 36
    property int thumbnailSlotHeight: 24
    property int sizeColumnWidth: 84
    property int createdColumnWidth: 120
    property int statusColumnWidth: 100
    property int progressColumnWidth: 150
    property int columnSpacing: 8
    property int contentPadding: 12
    property bool showSize: true
    property bool showCreated: true
    property bool playLoading: false
    property bool playing: false
    property bool folderLoading: false

    signal contextMenu(point position)
    signal openFolderRequested(string id)
    signal playRequested(string id, point position)

    readonly property var listView: ListView.view
    readonly property bool isActive: !row.settled
    readonly property bool showProgress: row.phase === "pending"
                                              || row.phase === "running"
                                              || row.phase === "paused"
    readonly property bool canOpenFolder: row.phase === "completed"
                                          && row.kind === "folder"
                                          && row.fileId.length > 0
    readonly property bool canPlay: row.phase === "completed" && row.playable
                                    && row.fileId.length > 0
    readonly property bool canActivate: (row.canOpenFolder || row.canPlay)
                                        && !row.folderLoading
    readonly property bool rowHighlighted: row.isActive
    readonly property var thumbnailEntry: ({
        id: row.fileId,
        name: row.name,
        kind: row.kind,
        hasThumbnail: row.hasThumbnail,
        video: row.video,
        playable: row.playable
    })

    readonly property string tone: {
        if (row.phase === "error")
            return "danger"
        if (row.phase === "completed")
            return "success"
        if (row.phase === "pending" || row.phase === "running")
            return "accent"
        return "neutral"
    }

    objectName: "taskRow_" + row.key
    implicitHeight: row.rowHeight
    leftPadding: 0
    rightPadding: 0
    topPadding: 0
    bottomPadding: 0

    KTheme { id: theme }

    HoverHandler {
        id: rowHover
    }

    function pointerOver(item, x, y) {
        const topLeft = item.mapToItem(row, 0, 0)
        return x >= topLeft.x && x <= topLeft.x + item.width
                && y >= topLeft.y && y <= topLeft.y + item.height
    }

    function activate(point) {
        if (row.folderLoading)
            return
        if (row.canOpenFolder) {
            row.openFolderRequested(row.fileId)
            return
        }
        if (row.canPlay)
            row.playRequested(row.fileId, point)
    }

    MouseArea {
        id: mouseArea

        anchors.fill: parent
        z: 1
        acceptedButtons: Qt.LeftButton | Qt.RightButton
        hoverEnabled: true
        cursorShape: row.canOpenFolder || row.canPlay
                     ? Qt.PointingHandCursor : Qt.ArrowCursor

        onClicked: function (mouse) {
            if (row.folderLoading)
                return
            if (row.listView)
                row.listView.forceActiveFocus()
            if (mouse.button === Qt.RightButton)
                row.contextMenu(Qt.point(mouse.x, mouse.y))
        }

        onDoubleClicked: function (mouse) {
            if (mouse.button === Qt.LeftButton)
                row.activate(Qt.point(mouse.x, mouse.y))
        }
    }

    VideoLoadingOverlay {
        objectName: "taskVideoLoading"

        x: row.contentPadding
        y: Math.round((row.rowHeight - row.thumbnailSlotHeight) / 2)
        width: row.thumbnailColumnWidth
        height: row.thumbnailSlotHeight
        compact: true
        cornerRadius: 3
        visible: row.playLoading
        z: 2
        onCancelRequested: {
            if (row.backend)
                row.backend.cancelPlayLoading()
        }
    }

    background: Rectangle {
        radius: theme.radiusSmall
        color: row.rowHighlighted
               ? theme.accentGhost
               : rowHover.hovered
                 ? theme.surfaceHover
                 : theme.surfaceClear

        Rectangle {
            objectName: "taskActiveAccent"
            visible: row.rowHighlighted
            width: 3
            height: Math.max(0, parent.height - 20)
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
            objectName: "taskThumbnailSlot"
            Layout.preferredWidth: row.thumbnailColumnWidth
            Layout.minimumWidth: row.thumbnailColumnWidth
            Layout.fillHeight: true

            FileThumbnail {
                objectName: "taskThumbnail"
                width: row.thumbnailColumnWidth
                height: row.thumbnailSlotHeight
                anchors.centerIn: parent
                backend: row.backend
                entry: row.thumbnailEntry
                iconPixelSize: 18
                videoBadgeSize: 16
                videoBadgeMargin: 2
                loading: row.playLoading
                playing: row.playing
            }
        }

        ElidedLabel {
            id: nameLabel

            objectName: "taskNameText"
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            Layout.fillHeight: true
            text: row.name
            color: theme.textPrimary
            font.pixelSize: theme.fontBody
            font.weight: Font.DemiBold
            verticalAlignment: Text.AlignVCenter
            elide: Text.ElideMiddle
            toolTipText: row.phase === "error" && row.detail.length > 0
                         ? row.name + "\n" + row.detail : row.name
            showToolTipWhenUntruncated: row.phase === "error"
                                        && row.detail.length > 0
            externallyHovered: mouseArea.containsMouse
                               && row.pointerOver(nameLabel, mouseArea.mouseX,
                                                  mouseArea.mouseY)
        }

        Item {
            id: statusSlot

            objectName: "taskStatusSlot"
            Layout.preferredWidth: row.statusColumnWidth
            Layout.minimumWidth: row.statusColumnWidth
            Layout.fillHeight: true

            KStatusBadge {
                objectName: "taskStatusBadge"
                anchors.centerIn: parent
                text: row.phaseText
                tone: row.tone
            }

            ToolTip.visible: row.phase === "error" && row.detail.length > 0
                             && mouseArea.containsMouse
                             && row.pointerOver(statusSlot, mouseArea.mouseX,
                                                mouseArea.mouseY)
            ToolTip.text: row.detail
        }

        Item {
            objectName: "taskProgressSlot"
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

        Label {
            objectName: "taskSizeText"
            visible: row.showSize
            Layout.preferredWidth: row.showSize ? row.sizeColumnWidth : 0
            Layout.minimumWidth: row.showSize ? row.sizeColumnWidth : 0
            Layout.fillHeight: true
            text: row.sizeText
            color: theme.textMuted
            font.pixelSize: theme.fontMeta
            horizontalAlignment: Text.AlignRight
            verticalAlignment: Text.AlignVCenter
        }

        Label {
            objectName: "taskCreatedText"
            visible: row.showCreated
            Layout.preferredWidth: row.showCreated ? row.createdColumnWidth : 0
            Layout.minimumWidth: row.showCreated ? row.createdColumnWidth : 0
            Layout.fillHeight: true
            text: row.createdText
            color: theme.textMuted
            font.pixelSize: theme.fontMeta
            horizontalAlignment: Text.AlignRight
            verticalAlignment: Text.AlignVCenter
        }
    }
}
