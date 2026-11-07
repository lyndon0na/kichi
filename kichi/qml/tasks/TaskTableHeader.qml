import QtQuick
import QtQuick.Layouts
import "../components"

// 离线任务表头。列宽、响应式显隐由 TasksPage 同时交给本组件与 TaskRow。
KTableHeader {
    id: header

    property int rowHeight: 34
    property int thumbnailColumnWidth: 36
    property int sizeColumnWidth: 84
    property int createdColumnWidth: 120
    property int statusColumnWidth: 100
    property int progressColumnWidth: 150
    property bool showSize: true
    property bool showCreated: true

    objectName: "tasksTableHeader"
    headerRowHeight: header.rowHeight
    columnSpacing: 8

    Item {
        Layout.preferredWidth: header.thumbnailColumnWidth
        Layout.minimumWidth: header.thumbnailColumnWidth
    }

    KHeaderCell {
        text: qsTr("任务名称")
        fillWidth: true
        minimumCellWidth: 0
        elideText: true
    }

    KHeaderCell {
        objectName: "tasksStatusHeader"
        text: qsTr("状态")
        fixedWidth: true
        cellWidth: header.statusColumnWidth
        minimumCellWidth: header.statusColumnWidth
        horizontalAlignment: Text.AlignHCenter
    }

    KHeaderCell {
        objectName: "tasksProgressHeader"
        text: qsTr("进度")
        fixedWidth: true
        cellWidth: header.progressColumnWidth
        minimumCellWidth: header.progressColumnWidth
        horizontalAlignment: Text.AlignHCenter
    }

    KHeaderCell {
        objectName: "tasksSizeHeader"
        visible: header.showSize
        text: qsTr("大小")
        fixedWidth: true
        cellWidth: header.showSize ? header.sizeColumnWidth : 0
        minimumCellWidth: header.showSize ? header.sizeColumnWidth : 0
        horizontalAlignment: Text.AlignRight
    }

    KHeaderCell {
        objectName: "tasksCreatedHeader"
        visible: header.showCreated
        text: qsTr("创建时间")
        fixedWidth: true
        cellWidth: header.showCreated ? header.createdColumnWidth : 0
        minimumCellWidth: header.showCreated ? header.createdColumnWidth : 0
        horizontalAlignment: Text.AlignRight
    }
}
