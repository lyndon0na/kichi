import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "../core"

// 设置页标准行：左边是设置名和一句说明，右边是控件。
// 卡片式设置里行直接铺在面板上，不加卡片、不画分隔线；上下各 16px 留白就是行距。
// 需要整行可点的行（例如展开高级项）把 interactive 打开即可。
Item {
    id: row

    property string title: ""
    property string description: ""
    property bool danger: false
    property bool interactive: false

    signal clicked()

    default property alias controlContent: controlHost.data

    readonly property KTheme theme: KTheme { parent: row }
    readonly property bool hovered: rowMouse.containsMouse && row.interactive

    Layout.fillWidth: true
    implicitHeight: content.implicitHeight + theme.gapLg * 2

    Rectangle {
        anchors.fill: parent
        color: row.hovered ? theme.surfaceHover : theme.surfaceClear
    }

    RowLayout {
        id: content
        anchors.fill: parent
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
                font.pixelSize: theme.fontSection
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
