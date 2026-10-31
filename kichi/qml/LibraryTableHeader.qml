import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 最近保存 / 播放历史 / 已加星标共用的静态表头。
Item {
    id: header

    property int thumbnailSlotWidth: 36
    property int primaryColumnWidth: 110
    property int secondaryColumnWidth: 150
    property int contentPadding: 12
    property int columnSpacing: 12
    property int headerRowHeight: 34
    property int headerPixelSize: 12
    property string primaryLabel: ""
    property string secondaryLabel: ""

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

        Label {
            Layout.preferredWidth: header.primaryColumnWidth
            text: header.primaryLabel
            color: theme.textMuted
            font.pixelSize: header.headerPixelSize
            font.weight: Font.DemiBold
            verticalAlignment: Text.AlignVCenter
            horizontalAlignment: Text.AlignRight
        }

        Label {
            Layout.preferredWidth: header.secondaryColumnWidth
            text: header.secondaryLabel
            color: theme.textMuted
            font.pixelSize: header.headerPixelSize
            font.weight: Font.DemiBold
            verticalAlignment: Text.AlignVCenter
            horizontalAlignment: Text.AlignRight
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
