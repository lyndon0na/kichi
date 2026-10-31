import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 回收站表格的一行：名称 / 删除时间 / 大小。
// 单击选择，右键打开操作菜单；双击不执行动作，避免误还原或误删。
ItemDelegate {
    id: row

    required property var entry
    required property int index
    property var backend: null
    readonly property var listView: ListView.view
    property int rowIndex: -1
    property int thumbnailSlotWidth: 36
    property int thumbnailSlotHeight: 24
    property int deletedColumnWidth: 150
    property int sizeColumnWidth: 96
    property int columnSpacing: 12
    property int rowHeight: 44
    property int textPixelSize: 14
    property int metaPixelSize: 12
    property int iconPixelSize: 16
    property int contentPadding: 12
    property bool rowHovered: false
    property bool rowPressed: false

    signal selectRequested(int modifiers, int index)
    signal contextMenu(point position)

    KTheme { id: theme }

    function pointerOver(item, x, y) {
        const topLeft = item.mapToItem(row, 0, 0)
        return x >= topLeft.x && x <= topLeft.x + item.width
                && y >= topLeft.y && y <= topLeft.y + item.height
    }

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

    MouseArea {
        id: mouseArea

        anchors.fill: parent
        z: 1
        acceptedButtons: Qt.LeftButton | Qt.RightButton
        hoverEnabled: true
        cursorShape: Qt.ArrowCursor
        onPressedChanged: row.rowPressed = pressed
        onContainsMouseChanged: row.rowHovered = containsMouse

        onClicked: function (mouse) {
            if (row.listView)
                row.listView.forceActiveFocus()
            if (mouse.button === Qt.RightButton) {
                if (row.entry)
                    row.contextMenu(Qt.point(mouse.x, mouse.y))
                return
            }
            row.selectRequested(mouse.modifiers, row.rowIndex)
        }
    }

    // 平铺行只靠底部分隔线分段，不用卡片圆角和行间空隙。
    background: Rectangle {
        color: row.highlighted
               ? theme.accentSoft
               : row.rowPressed
                 ? theme.surfacePressed
                 : row.rowHovered
                   ? theme.surfaceHover
                   : theme.surfaceClear

        Rectangle {
            objectName: "trashRowAccent"
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
            objectName: "trashRowDivider"
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
            objectName: "thumbnailSlot"
            Layout.preferredWidth: row.thumbnailSlotWidth
            Layout.fillHeight: true

            FileThumbnail {
                objectName: "fileThumbnail"
                width: row.thumbnailSlotWidth
                height: row.thumbnailSlotHeight
                anchors.centerIn: parent
                backend: row.backend
                entry: row.entry
                iconPixelSize: row.iconPixelSize
            }
        }

        ElidedLabel {
            id: nameLabel
            objectName: "trashNameText"
            Layout.fillWidth: true
            text: row.entry ? row.entry.name : ""
            font.pixelSize: row.textPixelSize
            color: palette.text
            verticalAlignment: Text.AlignVCenter
            elide: Text.ElideRight
            externallyHovered: row.rowHovered
                               && row.pointerOver(nameLabel, mouseArea.mouseX,
                                                  mouseArea.mouseY)
        }

        ElidedLabel {
            id: trashDeletedText
            objectName: "trashDeletedText"
            Layout.preferredWidth: row.deletedColumnWidth
            text: row.entry ? row.entry.deletedText : ""
            font.pixelSize: row.metaPixelSize
            color: palette.placeholderText
            verticalAlignment: Text.AlignVCenter
            horizontalAlignment: Text.AlignRight
            elide: Text.ElideRight
            externallyHovered: row.rowHovered
                               && row.pointerOver(trashDeletedText, mouseArea.mouseX,
                                                  mouseArea.mouseY)
        }

        Label {
            objectName: "trashSizeText"
            Layout.preferredWidth: row.sizeColumnWidth
            text: row.entry ? row.entry.sizeText : ""
            font.pixelSize: row.metaPixelSize
            color: palette.placeholderText
            verticalAlignment: Text.AlignVCenter
            horizontalAlignment: Text.AlignRight
        }
    }
}
