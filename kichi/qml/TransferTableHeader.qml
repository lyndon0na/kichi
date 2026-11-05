import QtQuick
import QtQuick.Layouts

// 传输任务页表头。列宽与 TransferMainRow 共用 TransferTableDensity，
// 响应式显隐也由页面同时传给两边，避免表头与内容错位。
KTableHeader {
    id: header

    property bool showSize: true
    property bool showRate: true

    objectName: "transferTableHeader"
    headerRowHeight: density.headerRowHeight
    contentPadding: density.rowPadding
    columnSpacing: density.columnSpacing

    TransferTableDensity { id: density }

    Item {
        Layout.preferredWidth: density.iconColumnWidth
        Layout.minimumWidth: density.iconColumnWidth
    }

    KHeaderCell {
        text: qsTr("名称")
        fillWidth: true
        minimumCellWidth: 0
    }

    KHeaderCell {
        text: qsTr("状态")
        fixedWidth: true
        cellWidth: density.statusColumnWidth
        minimumCellWidth: density.statusColumnWidth
        horizontalAlignment: Text.AlignHCenter
    }

    KHeaderCell {
        objectName: "transferProgressHeader"
        text: qsTr("进度")
        fixedWidth: true
        cellWidth: density.progressColumnWidth
        minimumCellWidth: density.progressColumnWidth
        horizontalAlignment: Text.AlignHCenter
    }

    KHeaderCell {
        objectName: "transferSizeHeader"
        visible: header.showSize
        text: qsTr("大小")
        fixedWidth: true
        cellWidth: header.showSize ? density.sizeColumnWidth : 0
        minimumCellWidth: header.showSize ? density.sizeColumnWidth : 0
        horizontalAlignment: Text.AlignRight
    }

    KHeaderCell {
        objectName: "transferRateHeader"
        visible: header.showRate
        text: qsTr("速度与剩余")
        fixedWidth: true
        cellWidth: header.showRate ? density.rateColumnWidth : 0
        minimumCellWidth: header.showRate ? density.rateColumnWidth : 0
        horizontalAlignment: Text.AlignRight
    }

    KHeaderCell {
        objectName: "transferActionHeader"
        text: qsTr("操作")
        fixedWidth: true
        cellWidth: density.actionColumnWidth
        minimumCellWidth: density.actionColumnWidth
        horizontalAlignment: Text.AlignHCenter
    }
}
