import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 离线任务表头。列宽、响应式显隐由 TasksPage 同时交给本组件与 TaskRow。
Item {
    id: header

    property int rowHeight: 34
    property int thumbnailColumnWidth: 36
    property int sizeColumnWidth: 84
    property int createdColumnWidth: 120
    property int statusColumnWidth: 100
    property int progressColumnWidth: 150
    property int columnSpacing: 8
    property int contentPadding: 12
    property bool showSize: true
    property bool showCreated: true

    objectName: "tasksTableHeader"
    implicitHeight: header.rowHeight

    KTheme { id: theme }

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: header.contentPadding
        anchors.rightMargin: header.contentPadding
        spacing: header.columnSpacing

        Item {
            Layout.preferredWidth: header.thumbnailColumnWidth
            Layout.minimumWidth: header.thumbnailColumnWidth
        }

        Label {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            text: qsTr("任务名称")
            color: theme.textMuted
            font.pixelSize: theme.fontMeta
            font.weight: Font.DemiBold
            verticalAlignment: Text.AlignVCenter
            elide: Text.ElideRight
        }

        Label {
            objectName: "tasksStatusHeader"
            Layout.preferredWidth: header.statusColumnWidth
            Layout.minimumWidth: header.statusColumnWidth
            text: qsTr("状态")
            color: theme.textMuted
            font.pixelSize: theme.fontMeta
            font.weight: Font.DemiBold
            horizontalAlignment: Text.AlignHCenter
            verticalAlignment: Text.AlignVCenter
        }

        Item {
            Layout.preferredWidth: header.progressColumnWidth
            Layout.minimumWidth: header.progressColumnWidth

            Label {
                objectName: "tasksProgressHeader"
                anchors.fill: parent
                text: qsTr("进度")
                color: theme.textMuted
                font.pixelSize: theme.fontMeta
                font.weight: Font.DemiBold
                horizontalAlignment: Text.AlignHCenter
                verticalAlignment: Text.AlignVCenter
            }
        }

        Label {
            objectName: "tasksSizeHeader"
            visible: header.showSize
            Layout.preferredWidth: header.showSize ? header.sizeColumnWidth : 0
            Layout.minimumWidth: header.showSize ? header.sizeColumnWidth : 0
            text: qsTr("大小")
            color: theme.textMuted
            font.pixelSize: theme.fontMeta
            font.weight: Font.DemiBold
            horizontalAlignment: Text.AlignRight
            verticalAlignment: Text.AlignVCenter
        }

        Label {
            objectName: "tasksCreatedHeader"
            visible: header.showCreated
            Layout.preferredWidth: header.showCreated ? header.createdColumnWidth : 0
            Layout.minimumWidth: header.showCreated ? header.createdColumnWidth : 0
            text: qsTr("创建时间")
            color: theme.textMuted
            font.pixelSize: theme.fontMeta
            font.weight: Font.DemiBold
            horizontalAlignment: Text.AlignRight
            verticalAlignment: Text.AlignVCenter
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
