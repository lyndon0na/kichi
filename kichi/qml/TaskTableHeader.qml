import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 离线任务表头。列宽由 TasksPage 同时交给本组件与 TaskRow，
// 两侧共用一组数值，避免表头和内容错位。
Item {
    id: header

    property int rowHeight: 34
    property int iconColumnWidth: 22
    property int statusColumnWidth: 100
    property int progressColumnWidth: 150
    property int actionColumnWidth: 96
    property int columnSpacing: 12
    property int contentPadding: 12

    objectName: "tasksTableHeader"
    implicitHeight: header.rowHeight

    UiMetrics { id: metrics }
    KTheme { id: theme }

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: header.contentPadding
        anchors.rightMargin: header.contentPadding + metrics.scrollBarSlot
        spacing: header.columnSpacing

        Item {
            Layout.preferredWidth: header.iconColumnWidth
            Layout.minimumWidth: header.iconColumnWidth
        }

        Label {
            Layout.fillWidth: true
            text: qsTr("任务名称")
            color: theme.textMuted
            font.pixelSize: theme.fontMeta
            font.weight: Font.DemiBold
            verticalAlignment: Text.AlignVCenter
        }

        Item {
            Layout.preferredWidth: header.statusColumnWidth
            Layout.minimumWidth: header.statusColumnWidth

            Label {
                anchors.fill: parent
                text: qsTr("状态")
                color: theme.textMuted
                font.pixelSize: theme.fontMeta
                font.weight: Font.DemiBold
                horizontalAlignment: Text.AlignHCenter
                verticalAlignment: Text.AlignVCenter
            }
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

        Item {
            Layout.preferredWidth: header.actionColumnWidth
            Layout.minimumWidth: header.actionColumnWidth

            Label {
                anchors.fill: parent
                text: qsTr("操作")
                color: theme.textMuted
                font.pixelSize: theme.fontMeta
                font.weight: Font.DemiBold
                horizontalAlignment: Text.AlignHCenter
                verticalAlignment: Text.AlignVCenter
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
