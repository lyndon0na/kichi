import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 回收站表头。列宽由 TrashPage 同时下发给本组件与 TrashRow。
Item {
    id: header

    property int iconColumnWidth: 20
    property int deletedColumnWidth: 150
    property int sizeColumnWidth: 96
    property int contentPadding: 12
    property int columnSpacing: 12
    property int headerRowHeight: 32
    property int headerPixelSize: 12

    UiMetrics { id: metrics }
    KTheme { id: theme }

    implicitHeight: header.headerRowHeight + 1

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        RowLayout {
            Layout.fillWidth: true
            Layout.leftMargin: header.contentPadding
            Layout.rightMargin: header.contentPadding + metrics.scrollBarSlot
            Layout.preferredHeight: header.headerRowHeight
            spacing: header.columnSpacing

            Item {
                Layout.preferredWidth: header.iconColumnWidth
                Layout.preferredHeight: 1
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

                Rectangle {
                    width: 1
                    height: Math.round(header.headerRowHeight * 0.45)
                    anchors.left: parent.left
                    anchors.verticalCenter: parent.verticalCenter
                    color: theme.divider
                }

                Label {
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

                Rectangle {
                    width: 1
                    height: Math.round(header.headerRowHeight * 0.45)
                    anchors.left: parent.left
                    anchors.verticalCenter: parent.verticalCenter
                    color: theme.divider
                }

                Label {
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
            Layout.fillWidth: true
            Layout.preferredHeight: 1
            color: theme.hairline
        }
    }
}
