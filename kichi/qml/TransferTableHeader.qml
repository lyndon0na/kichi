import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 传输任务页表头。列宽与 TransferMainRow 共用 TransferTableDensity，
// 响应式显隐也由页面同时传给两边，避免表头与内容错位。
Item {
    id: header

    property bool showSize: true
    property bool showRate: true

    objectName: "transferTableHeader"
    implicitHeight: density.headerRowHeight

    TransferTableDensity { id: density }
    KTheme { id: theme }

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: density.rowPadding
        anchors.rightMargin: density.rowPadding
        spacing: density.columnSpacing

        Item {
            Layout.preferredWidth: density.iconColumnWidth
            Layout.minimumWidth: density.iconColumnWidth
        }

        Label {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            text: qsTr("名称")
            color: theme.textMuted
            font.pixelSize: theme.fontMeta
            font.weight: Font.DemiBold
            verticalAlignment: Text.AlignVCenter
        }

        Label {
            Layout.preferredWidth: density.statusColumnWidth
            Layout.minimumWidth: density.statusColumnWidth
            text: qsTr("状态")
            color: theme.textMuted
            font.pixelSize: theme.fontMeta
            font.weight: Font.DemiBold
            horizontalAlignment: Text.AlignHCenter
            verticalAlignment: Text.AlignVCenter
        }

        Label {
            objectName: "transferProgressHeader"
            Layout.preferredWidth: density.progressColumnWidth
            Layout.minimumWidth: density.progressColumnWidth
            text: qsTr("进度")
            color: theme.textMuted
            font.pixelSize: theme.fontMeta
            font.weight: Font.DemiBold
            horizontalAlignment: Text.AlignHCenter
            verticalAlignment: Text.AlignVCenter
        }

        Label {
            objectName: "transferSizeHeader"
            visible: header.showSize
            Layout.preferredWidth: header.showSize ? density.sizeColumnWidth : 0
            Layout.minimumWidth: header.showSize ? density.sizeColumnWidth : 0
            text: qsTr("大小")
            color: theme.textMuted
            font.pixelSize: theme.fontMeta
            font.weight: Font.DemiBold
            horizontalAlignment: Text.AlignRight
            verticalAlignment: Text.AlignVCenter
        }

        Label {
            objectName: "transferRateHeader"
            visible: header.showRate
            Layout.preferredWidth: header.showRate ? density.rateColumnWidth : 0
            Layout.minimumWidth: header.showRate ? density.rateColumnWidth : 0
            text: qsTr("速度与剩余")
            color: theme.textMuted
            font.pixelSize: theme.fontMeta
            font.weight: Font.DemiBold
            horizontalAlignment: Text.AlignRight
            verticalAlignment: Text.AlignVCenter
        }

        Label {
            objectName: "transferActionHeader"
            Layout.preferredWidth: density.actionColumnWidth
            Layout.minimumWidth: density.actionColumnWidth
            text: qsTr("操作")
            color: theme.textMuted
            font.pixelSize: theme.fontMeta
            font.weight: Font.DemiBold
            horizontalAlignment: Text.AlignHCenter
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
