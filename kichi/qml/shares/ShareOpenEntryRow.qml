pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "../components"
import "../core"
import "../files"

// 分享内容里的一行：文件点整行切换勾选；文件夹点整行进入，
// 左侧方块也能单独勾选整个文件夹。
Item {
    id: row

    required property string id
    required property string name
    required property string kind
    required property bool isFolder
    required property string sizeText
    property bool selected: false

    signal toggleRequested()
    signal enterRequested()

    KTheme { id: theme }

    implicitHeight: 44

    Rectangle {
        anchors.fill: parent
        radius: theme.radiusCard
        color: mouse.containsMouse ? theme.surfaceHover : "transparent"
    }

    MouseArea {
        id: mouse
        anchors.fill: parent
        hoverEnabled: true

        onClicked: {
            if (row.isFolder)
                row.enterRequested()
            else
                row.toggleRequested()
        }
    }

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: 4
        anchors.rightMargin: 10
        spacing: theme.gapSm

        // 勾选方块：和整行点击分开，文件夹也能被选中转存。
        Item {
            Layout.preferredWidth: 28
            Layout.fillHeight: true

            Rectangle {
                anchors.centerIn: parent
                width: 16
                height: 16
                radius: 4
                color: row.selected ? theme.accent : "transparent"
                border.width: 1
                border.color: row.selected ? theme.accent : theme.textMuted

                NavIcon {
                    anchors.centerIn: parent
                    visible: row.selected
                    width: 11
                    height: 11
                    name: "check"
                    color: theme.accentForeground
                }
            }

            MouseArea {
                anchors.fill: parent
                onClicked: row.toggleRequested()
            }
        }

        FileIcon {
            Layout.preferredWidth: 18
            Layout.preferredHeight: 18
            kind: row.kind
            fileName: row.name
            color: theme.textSecondary
        }

        ElidedLabel {
            Layout.fillWidth: true
            text: row.name
            color: theme.textPrimary
            font.pixelSize: theme.fontBody
            elide: Text.ElideMiddle
            verticalAlignment: Text.AlignVCenter
        }

        ElidedLabel {
            visible: !row.isFolder
            Layout.preferredWidth: 84
            text: row.sizeText
            color: theme.textMuted
            font.pixelSize: theme.fontMeta
            horizontalAlignment: Text.AlignRight
            verticalAlignment: Text.AlignVCenter
        }

        ElidedLabel {
            visible: row.isFolder
            text: qsTr("进入")
            color: theme.textMuted
            font.pixelSize: theme.fontMeta
            verticalAlignment: Text.AlignVCenter
        }

        NavIcon {
            visible: row.isFolder
            Layout.preferredWidth: 12
            Layout.preferredHeight: 12
            name: "chevron-right"
            color: theme.textMuted
        }
    }
}
