import QtQuick
import QtQuick.Layouts

// 分享表格的表头。列宽由 SharesPage 同时交给本组件与 ShareRow，
// 两侧共用同一组数值，避免表头和内容错位。
KTableHeader {
    id: header

    property int rowHeight: 34
    property int iconColumnWidth: 22
    property int statusColumnWidth: 100
    property int expirationColumnWidth: 64
    property int createdColumnWidth: 120
    property int actionColumnWidth: 96

    objectName: "sharesTableHeader"
    headerRowHeight: header.rowHeight

    Item {
        Layout.preferredWidth: header.iconColumnWidth
        Layout.minimumWidth: header.iconColumnWidth
    }

    KHeaderCell {
        text: qsTr("名称")
        fillWidth: true
    }

    KHeaderCell {
        text: qsTr("状态")
        fixedWidth: true
        cellWidth: header.statusColumnWidth
        minimumCellWidth: header.statusColumnWidth
        horizontalAlignment: Text.AlignHCenter
    }

    KHeaderCell {
        text: qsTr("有效期")
        fixedWidth: true
        cellWidth: header.expirationColumnWidth
        minimumCellWidth: header.expirationColumnWidth
        horizontalAlignment: Text.AlignHCenter
    }

    KHeaderCell {
        text: qsTr("创建时间")
        fixedWidth: true
        cellWidth: header.createdColumnWidth
        minimumCellWidth: header.createdColumnWidth
        horizontalAlignment: Text.AlignRight
    }

    KHeaderCell {
        text: qsTr("操作")
        fixedWidth: true
        cellWidth: header.actionColumnWidth
        minimumCellWidth: header.actionColumnWidth
        horizontalAlignment: Text.AlignHCenter
    }
}
