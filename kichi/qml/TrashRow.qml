import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 回收站表格的一行：名称 / 删除时间 / 大小。
// 单击选择，右键打开操作菜单；双击不执行动作，避免误还原或误删。
ItemDelegate {
    id: row

    property var entry: null
    readonly property var listView: ListView.view
    property int rowIndex: -1
    property int iconColumnWidth: 20
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

    implicitHeight: row.rowHeight
    leftPadding: row.contentPadding
    rightPadding: row.contentPadding

    icon.name: ""
    icon.source: ""
    icon.width: 0
    icon.height: 0
    icon.color: "transparent"

    MouseArea {
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

    background: Rectangle {
        radius: theme.radiusControl
        color: row.highlighted
               ? theme.accentSoft
               : row.rowPressed
                 ? theme.surfacePressed
                 : row.rowHovered
                   ? theme.surfaceHover
                   : theme.surfaceClear

        Rectangle {
            visible: row.highlighted
            width: 3
            height: parent.height - 20
            radius: 1.5
            anchors.left: parent.left
            anchors.leftMargin: 1
            anchors.verticalCenter: parent.verticalCenter
            color: theme.accent
        }
    }

    contentItem: RowLayout {
        spacing: row.columnSpacing

        Item {
            Layout.preferredWidth: row.iconColumnWidth
            Layout.fillHeight: true

            FileIcon {
                width: Math.min(row.iconPixelSize, row.iconColumnWidth)
                height: width
                anchors.centerIn: parent
                kind: row.entry ? row.entry.kind : ""
                fileName: row.entry ? row.entry.name : ""
                color: palette.text
                opacity: row.entry && row.entry.kind === "folder" ? 1.0 : 0.6
            }
        }

        Label {
            Layout.fillWidth: true
            text: row.entry ? row.entry.name : ""
            font.pixelSize: row.textPixelSize
            color: palette.text
            verticalAlignment: Text.AlignVCenter
            elide: Text.ElideRight
        }

        Label {
            Layout.preferredWidth: row.deletedColumnWidth
            text: row.entry ? row.entry.deletedText : ""
            font.pixelSize: row.metaPixelSize
            color: palette.placeholderText
            verticalAlignment: Text.AlignVCenter
            horizontalAlignment: Text.AlignRight
            elide: Text.ElideRight
        }

        Label {
            Layout.preferredWidth: row.sizeColumnWidth
            text: row.entry ? row.entry.sizeText : ""
            font.pixelSize: row.metaPixelSize
            color: palette.placeholderText
            verticalAlignment: Text.AlignVCenter
            horizontalAlignment: Text.AlignRight
        }
    }
}
