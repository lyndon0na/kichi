import QtQuick
import QtQuick.Controls

// 播放历史的时间分区带：列表和网格共用同一视觉。
Item {
    id: section

    property string title: ""
    property int contentPadding: 12
    property int rightPadding: 12

    implicitHeight: 38

    KTheme { id: theme }

    Rectangle {
        id: accentBar

        x: section.contentPadding
        width: 3
        height: 14
        radius: 1.5
        anchors.verticalCenter: parent.verticalCenter
        color: theme.accent
    }

    Label {
        id: titleLabel

        x: accentBar.x + accentBar.width + theme.gapSm
        height: parent.height
        text: section.title
        color: theme.textPrimary
        font.pixelSize: theme.fontMeta
        font.weight: Font.DemiBold
        verticalAlignment: Text.AlignVCenter
    }

    Rectangle {
        x: titleLabel.x + titleLabel.implicitWidth + theme.gapSm
        width: Math.max(0, parent.width - x - section.rightPadding)
        height: 1
        anchors.verticalCenter: parent.verticalCenter
        color: theme.hairline
    }
}
