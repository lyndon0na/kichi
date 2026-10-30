import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 文件页网格视图的一个条目。
//
// 与 FileRow 共用 FileThumbnail / FileSelection / FilePageInteraction：
// 单击多选、双击进入文件夹或播放、右键菜单都由 FilesPage 接到同一套逻辑上。
// 这里只负责网格卡片的外观，不另做一套文件操作。
ItemDelegate {
    id: card

    property var backend: null
    property var entry: null
    readonly property var gridView: GridView.view
    property int rowIndex: -1
    property bool selected: false

    // 数值由 FilesPage 从 FileListDensity 下发，和网格列宽一起计算。
    property int contentPadding: 5
    property int cardSpacing: 7
    property int nameLineHeight: 19
    property int nameBlockHeight: 38
    property int thumbnailRadius: 8
    property int textPixelSize: 14
    property int iconPixelSize: 28
    property int videoBadgeSize: 18

    signal contextMenu(point position)
    signal playRequested(string id, point position)
    signal selectRequested(int modifiers, int index)
    signal folderOpenRequested(string id)

    readonly property real thumbnailWidth: Math.max(1, width - contentPadding * 2)
    readonly property int thumbnailHeight: Math.max(1, Math.ceil(thumbnailWidth * 2 / 3))
    readonly property int cardHeight: contentPadding * 2 + thumbnailHeight + cardSpacing
                                      + nameBlockHeight

    leftPadding: contentPadding
    rightPadding: contentPadding
    topPadding: contentPadding
    bottomPadding: contentPadding
    implicitHeight: cardHeight

    icon.name: ""
    icon.source: ""
    icon.width: 0
    icon.height: 0
    icon.color: "transparent"

    KTheme { id: theme }

    background: Rectangle {
        radius: theme.radiusCard
        color: card.selected
               ? theme.accentSoft
               : mouseArea.pressed
                 ? theme.surfacePressed
                 : mouseArea.containsMouse
                   ? theme.surfaceHover
                   : theme.surfaceClear
    }

    MouseArea {
        id: mouseArea

        anchors.fill: parent
        z: 1
        acceptedButtons: Qt.LeftButton | Qt.RightButton
        hoverEnabled: true
        cursorShape: card.entry && card.entry.kind === "folder"
                     ? Qt.PointingHandCursor : Qt.ArrowCursor

        onClicked: function (mouse) {
            if (card.gridView)
                card.gridView.forceActiveFocus()
            if (mouse.button === Qt.RightButton) {
                if (card.entry)
                    card.contextMenu(Qt.point(mouse.x, mouse.y))
                return
            }
            card.selectRequested(mouse.modifiers, card.rowIndex)
        }

        onDoubleClicked: function (mouse) {
            if (mouse.button === Qt.LeftButton)
                card.activate(Qt.point(mouse.x, mouse.y))
        }
    }

    function activate(point) {
        if (!card.backend || !card.entry)
            return
        if (card.entry.kind === "folder") {
            card.folderOpenRequested(card.entry.id)
            return
        }
        if (card.entry.playable === true) {
            card.playRequested(card.entry.id, point)
            return
        }
        card.backend.downloadEntry(card.entry.id)
    }

    function pointerOver(item, x, y) {
        const topLeft = item.mapToItem(card, 0, 0)
        return x >= topLeft.x && x <= topLeft.x + item.width
                && y >= topLeft.y && y <= topLeft.y + item.height
    }

    contentItem: ColumnLayout {
        spacing: 0

        Item {
            id: thumbnailBox
            objectName: "gridThumbnailBox"

            Layout.fillWidth: true
            Layout.preferredHeight: card.thumbnailHeight

            Rectangle {
                objectName: "gridThumbnailFrame"

                anchors.fill: parent
                radius: card.thumbnailRadius
                color: theme.surfaceRaised
                border.width: 1
                border.color: theme.hairline
            }

            FileThumbnail {
                objectName: "gridThumbnail"

                anchors.fill: parent
                radius: card.thumbnailRadius
                videoBadgeSize: card.videoBadgeSize
                backend: card.backend
                entry: card.entry
                iconPixelSize: card.iconPixelSize
            }

            // 网格没有整行左侧的 3px 竖条，选中态用同色细环 + 卡片淡底，
            // 保持和列表同一套强调色语言。
            Rectangle {
                objectName: "gridSelectionRing"

                anchors.fill: parent
                anchors.margins: 1
                radius: card.thumbnailRadius
                color: "transparent"
                border.width: card.selected ? 2 : 0
                border.color: theme.accent
            }

            KStatusBadge {
                objectName: "gridAuditBadge"

                anchors.top: parent.top
                anchors.right: parent.right
                anchors.margins: 6
                z: 2
                text: qsTr("违规")
                tone: "danger"
                visible: card.entry ? card.entry.auditFlag === true : false
            }
        }

        Item { Layout.preferredHeight: card.cardSpacing }

        ElidedLabel {
            id: gridName
            objectName: "gridName"

            Layout.fillWidth: true
            Layout.preferredHeight: card.nameBlockHeight
            text: card.entry ? card.entry.name : ""
            font.pixelSize: card.textPixelSize
            font.weight: card.entry && card.entry.kind === "folder"
                         ? Font.DemiBold : Font.Normal
            color: theme.textPrimary
            wrapMode: Text.WrapAnywhere
            maximumLineCount: 2
            elide: Text.ElideRight
            lineHeight: card.nameLineHeight
            lineHeightMode: Text.FixedHeight
            verticalAlignment: Text.AlignTop
            externallyHovered: mouseArea.containsMouse
                               && card.pointerOver(gridName, mouseArea.mouseX,
                                                   mouseArea.mouseY)
        }
    }
}
