import QtQuick
import QtQuick.Layouts

// 回收站表头。列宽由 TrashPage 同时下发给本组件与 TrashRow。
KTableHeader {
    id: header

    property int thumbnailSlotWidth: 36
    property int deletedColumnWidth: 150
    property int sizeColumnWidth: 96
    property int headerPixelSize: 12

    headerRowHeight: 32

    Item {
        Layout.preferredWidth: header.thumbnailSlotWidth
        Layout.fillHeight: true
    }

    KHeaderCell {
        text: qsTr("名称")
        fillWidth: true
        font.pixelSize: header.headerPixelSize
    }

    KHeaderCell {
        objectName: "trashHeaderDeleted"
        text: qsTr("删除时间")
        fixedWidth: true
        cellWidth: header.deletedColumnWidth
        minimumCellWidth: header.deletedColumnWidth
        horizontalAlignment: Text.AlignRight
        font.pixelSize: header.headerPixelSize
    }

    KHeaderCell {
        objectName: "trashHeaderSize"
        text: qsTr("大小")
        fixedWidth: true
        cellWidth: header.sizeColumnWidth
        minimumCellWidth: header.sizeColumnWidth
        horizontalAlignment: Text.AlignRight
        font.pixelSize: header.headerPixelSize
    }
}
