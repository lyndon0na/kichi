import QtQuick
import QtQuick.Layouts
import "../components"

// 最近保存 / 播放历史 / 已加星标共用的静态表头。
KTableHeader {
    id: header

    property int thumbnailSlotWidth: 36
    property int primaryColumnWidth: 110
    property int secondaryColumnWidth: 150
    property int headerPixelSize: 12
    property string primaryLabel: ""
    property string secondaryLabel: ""

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
        text: header.primaryLabel
        fixedWidth: true
        cellWidth: header.primaryColumnWidth
        minimumCellWidth: header.primaryColumnWidth
        horizontalAlignment: Text.AlignRight
        font.pixelSize: header.headerPixelSize
    }

    KHeaderCell {
        text: header.secondaryLabel
        fixedWidth: true
        cellWidth: header.secondaryColumnWidth
        minimumCellWidth: header.secondaryColumnWidth
        horizontalAlignment: Text.AlignRight
        font.pixelSize: header.headerPixelSize
    }
}
