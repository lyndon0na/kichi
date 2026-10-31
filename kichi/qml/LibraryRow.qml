import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 资料库三页共用的一行：缩略图 / 名称 / 两个页面自定义信息列。
ItemDelegate {
    id: row

    property var entry: null
    property var backend: null
    readonly property var listView: ListView.view
    property int rowIndex: -1
    property bool playLoading: false
    property bool playing: false
    readonly property bool showPlayingState: row.playing && !row.playLoading

    property int thumbnailSlotWidth: 36
    property int thumbnailSlotHeight: 24
    property int primaryColumnWidth: 110
    property int secondaryColumnWidth: 150
    property int columnSpacing: 12
    property int rowHeight: 48
    property int textPixelSize: 14
    property int metaPixelSize: 12
    property int iconPixelSize: 16
    property int videoBadgeSize: 16
    property int videoBadgeMargin: 2
    property int contentPadding: 12
    property bool rowHovered: false
    property bool rowPressed: false

    signal activateRequested(string id)
    signal playRequested(string id, point position)

    KTheme { id: theme }

    implicitHeight: row.rowHeight
    leftPadding: row.contentPadding
    rightPadding: row.contentPadding
    topPadding: 0
    bottomPadding: 0

    icon.name: ""
    icon.source: ""
    icon.width: 0
    icon.height: 0
    icon.color: "transparent"

    function activate(point) {
        if (!row.backend || !row.entry)
            return
        if (row.entry.playable === true)
            row.playRequested(row.entry.id, point)
        else
            row.activateRequested(row.entry.id)
    }

    function pointerOver(item, x, y) {
        const topLeft = item.mapToItem(row, 0, 0)
        return x >= topLeft.x && x <= topLeft.x + item.width
                && y >= topLeft.y && y <= topLeft.y + item.height
    }

    MouseArea {
        id: mouseArea

        anchors.fill: parent
        z: 1
        acceptedButtons: Qt.LeftButton
        hoverEnabled: true
        cursorShape: row.entry && row.entry.playable === true
                     ? Qt.PointingHandCursor : Qt.ArrowCursor
        onPressedChanged: row.rowPressed = pressed
        onContainsMouseChanged: row.rowHovered = containsMouse

        onClicked: {
            if (row.listView) {
                row.listView.currentIndex = row.rowIndex
                row.listView.forceActiveFocus()
            }
        }

        onDoubleClicked: function (mouse) {
            if (mouse.button === Qt.LeftButton)
                row.activate(Qt.point(mouse.x, mouse.y))
        }
    }

    VideoLoadingOverlay {
        objectName: "libraryVideoLoading"

        x: row.leftPadding
        y: Math.round((row.rowHeight - row.thumbnailSlotHeight) / 2)
        width: row.thumbnailSlotWidth
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
        color: row.highlighted
               ? theme.accentSoft
               : row.rowPressed
                 ? theme.surfacePressed
                 : row.rowHovered
                   ? theme.surfaceHover
                   : theme.surfaceClear

        Rectangle {
            objectName: "libraryRowAccent"
            visible: row.highlighted
            width: 3
            height: Math.max(0, parent.height - 28)
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
        spacing: row.columnSpacing

        Item {
            Layout.preferredWidth: row.thumbnailSlotWidth
            Layout.fillHeight: true

            FileThumbnail {
                width: row.thumbnailSlotWidth
                height: row.thumbnailSlotHeight
                anchors.centerIn: parent
                backend: row.backend
                entry: row.entry
                iconPixelSize: row.iconPixelSize
                videoBadgeSize: row.videoBadgeSize
                videoBadgeMargin: row.videoBadgeMargin
                loading: row.playLoading
                playing: row.showPlayingState
            }
        }

        Row {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 4

            ElidedLabel {
                id: nameLabel
                width: Math.min(implicitWidth,
                                Math.max(0, parent.width - playingBadge.width - parent.spacing))
                height: parent.height
                text: row.entry ? row.entry.name : ""
                font.pixelSize: row.textPixelSize
                color: palette.text
                verticalAlignment: Text.AlignVCenter
                elide: Text.ElideRight
                externallyHovered: row.rowHovered
                                   && row.pointerOver(nameLabel, mouseArea.mouseX,
                                                      mouseArea.mouseY)
            }

            Rectangle {
                id: playingBadge

                visible: row.showPlayingState
                width: visible ? implicitWidth : 0
                implicitWidth: playingContent.implicitWidth + 10
                height: 18
                y: Math.round((parent.height - height) / 2)
                radius: theme.radiusSmall
                color: theme.accentSoft
                Accessible.name: qsTr("播放中")

                RowLayout {
                    id: playingContent
                    anchors.centerIn: parent
                    spacing: 4

                    KPlayingBars {
                        implicitWidth: 10
                        implicitHeight: 10
                        fillColor: theme.accent
                        running: playingBadge.visible
                    }

                    Label {
                        text: qsTr("播放中")
                        font.pixelSize: theme.fontCaption
                        font.weight: Font.DemiBold
                        color: theme.accent
                    }
                }
            }
        }

        ElidedLabel {
            Layout.preferredWidth: row.primaryColumnWidth
            text: row.entry ? row.entry.primaryText : ""
            font.pixelSize: row.metaPixelSize
            color: palette.placeholderText
            verticalAlignment: Text.AlignVCenter
            horizontalAlignment: Text.AlignRight
            elide: Text.ElideRight
            externallyHovered: row.rowHovered
                               && row.pointerOver(this, mouseArea.mouseX, mouseArea.mouseY)
        }

        ElidedLabel {
            Layout.preferredWidth: row.secondaryColumnWidth
            text: row.entry ? row.entry.secondaryText : ""
            font.pixelSize: row.metaPixelSize
            color: palette.placeholderText
            verticalAlignment: Text.AlignVCenter
            horizontalAlignment: Text.AlignRight
            elide: Text.ElideRight
            externallyHovered: row.rowHovered
                               && row.pointerOver(this, mouseArea.mouseX, mouseArea.mouseY)
        }
    }
}
