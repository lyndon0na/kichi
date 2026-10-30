import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 回收站表头。列宽由 TrashPage 同时下发给本组件与 TrashRow。
Item {
    id: header

    property int thumbnailSlotWidth: 36
    property int deletedColumnWidth: 150
    property int sizeColumnWidth: 96
    property int contentPadding: 12
    property int columnSpacing: 12
    property int headerRowHeight: 32
    property int headerPixelSize: 12

    UiMetrics { id: metrics }
    KTheme { id: theme }

    implicitHeight: header.headerRowHeight

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: header.contentPadding
        anchors.rightMargin: header.contentPadding + metrics.scrollBarSlot
        spacing: header.columnSpacing

        Item {
            Layout.preferredWidth: header.thumbnailSlotWidth
            Layout.fillHeight: true
        }

        Label {
            Layout.fillWidth: true
            text: qsTr("名称")
            color: theme.textMuted
            font.pixelSize: header.headerPixelSize
            font.weight: Font.DemiBold
            verticalAlignment: Text.AlignVCenter
        }

        Item {
            Layout.preferredWidth: header.deletedColumnWidth
            Layout.fillHeight: true

            Label {
                objectName: "trashHeaderDeleted"
                anchors.fill: parent
                text: qsTr("删除时间")
                color: theme.textMuted
                font.pixelSize: header.headerPixelSize
                font.weight: Font.DemiBold
                verticalAlignment: Text.AlignVCenter
                horizontalAlignment: Text.AlignRight
            }
        }

        Item {
            Layout.preferredWidth: header.sizeColumnWidth
            Layout.fillHeight: true

            Label {
                objectName: "trashHeaderSize"
                anchors.fill: parent
                text: qsTr("大小")
                color: theme.textMuted
                font.pixelSize: header.headerPixelSize
                font.weight: Font.DemiBold
                verticalAlignment: Text.AlignVCenter
                horizontalAlignment: Text.AlignRight
            }
        }
    }

    Rectangle {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        height: 1
        color: theme.hairline
    }
}
