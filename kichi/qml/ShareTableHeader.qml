import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 分享表格的表头。列宽由 SharesPage 同时交给本组件与 ShareRow，
// 两侧共用同一组数值，避免表头和内容错位。
Item {
    id: header

    property int rowHeight: 34
    property int iconColumnWidth: 22
    property int statusColumnWidth: 100
    property int expirationColumnWidth: 64
    property int createdColumnWidth: 120
    property int actionColumnWidth: 96
    property int columnSpacing: 12
    property int contentPadding: 12

    objectName: "sharesTableHeader"
    implicitHeight: header.rowHeight

    KTheme { id: theme }

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: header.contentPadding
        anchors.rightMargin: header.contentPadding
        spacing: header.columnSpacing

        Item {
            Layout.preferredWidth: header.iconColumnWidth
            Layout.minimumWidth: header.iconColumnWidth
        }

        Label {
            Layout.fillWidth: true
            text: qsTr("名称")
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
            Layout.preferredWidth: header.expirationColumnWidth
            Layout.minimumWidth: header.expirationColumnWidth

            Label {
                anchors.fill: parent
                text: qsTr("有效期")
                color: theme.textMuted
                font.pixelSize: theme.fontMeta
                font.weight: Font.DemiBold
                horizontalAlignment: Text.AlignHCenter
                verticalAlignment: Text.AlignVCenter
            }
        }

        Item {
            Layout.preferredWidth: header.createdColumnWidth
            Layout.minimumWidth: header.createdColumnWidth

            Label {
                anchors.fill: parent
                text: qsTr("创建时间")
                color: theme.textMuted
                font.pixelSize: theme.fontMeta
                font.weight: Font.DemiBold
                horizontalAlignment: Text.AlignRight
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
