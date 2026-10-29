import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 设置页标准行：左边是设置名和一句说明，右边是控件。
// 需要整行可点的行（例如展开高级项）把 interactive 打开即可。
Item {
    id: row

    property string title: ""
    property string description: ""
    property bool danger: false
    property bool divider: false
    property bool interactive: false

    signal clicked()

    default property alias controlContent: controlHost.data

    readonly property KTheme theme: KTheme { }
    readonly property bool hovered: rowMouse.containsMouse && row.interactive

    Layout.fillWidth: true
    implicitHeight: content.implicitHeight + theme.gapLg * 2

    Rectangle {
        anchors.fill: parent
        color: row.hovered ? theme.surfaceHover : theme.surfaceClear
    }

    Rectangle {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.leftMargin: theme.gapLg
        anchors.rightMargin: theme.gapLg
        height: 1
        visible: row.divider
        color: theme.divider
    }

    RowLayout {
        id: content
        anchors.fill: parent
        anchors.leftMargin: theme.gapLg
        anchors.rightMargin: theme.gapLg
        anchors.topMargin: theme.gapLg
        anchors.bottomMargin: theme.gapLg
        spacing: theme.gapXl

        ColumnLayout {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            spacing: theme.gapXs

            Label {
                Layout.fillWidth: true
                text: row.title
                font.pixelSize: theme.fontBody
                font.weight: Font.Medium
                color: row.danger ? theme.danger : theme.textPrimary
                wrapMode: Text.Wrap
            }

            Label {
                Layout.fillWidth: true
                visible: row.description.length > 0
                text: row.description
                font.pixelSize: theme.fontMeta
                color: theme.textMuted
                wrapMode: Text.Wrap
            }
        }

        Item {
            id: controlHost
            Layout.alignment: Qt.AlignVCenter
            implicitWidth: childrenRect.width
            implicitHeight: childrenRect.height
        }
    }

    MouseArea {
        id: rowMouse
        anchors.fill: parent
        enabled: row.interactive
        hoverEnabled: row.interactive
        acceptedButtons: Qt.LeftButton
        onClicked: row.clicked()
    }
}
