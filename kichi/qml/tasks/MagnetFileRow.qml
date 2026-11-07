pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "../components"
import "../core"
import "../files"

// 磁链文件树的一行：文件夹（可折叠、三态勾选）或普通文件。
//
// 勾选状态完全由模型角色驱动（不再依赖 CheckDelegate 的 checked），
// 这样「全选」等批量操作一定能反映到界面上。点击整行请求切换选中，
// 点左侧箭头只展开 / 收起文件夹。
ItemDelegate {
    id: row

    required property int index
    required property bool isFolder
    required property string nodeKey
    required property string parentKey
    required property int depth
    required property string nodeName
    required property string nodeSize
    required property string nodeNote
    required property bool available
    required property bool selected
    required property int nodeState
    required property bool expanded
    required property bool shown
    required property int fileIndex

    signal toggleRequested(int rowIndex)
    signal expandRequested(int rowIndex)

    readonly property KTheme theme: KTheme { parent: row }

    width: ListView.view ? ListView.view.width : implicitWidth
    height: shown ? 48 : 0
    visible: shown
    // ListView 会接管 delegate 的 visible；折叠行靠高度归零 + 透明双保险。
    opacity: shown ? 1 : 0
    enabled: row.isFolder || row.available
    hoverEnabled: true
    onClicked: row.toggleRequested(row.index)

    contentItem: RowLayout {
        spacing: row.theme.gapSm
        opacity: row.isFolder || row.available ? 1 : 0.6

        Item { Layout.preferredWidth: row.depth * 16 }

        // 展开箭头：占位固定宽度，保证文件夹与文件行的列对齐。
        Item {
            Layout.preferredWidth: 16
            Layout.preferredHeight: 16
            Layout.alignment: Qt.AlignVCenter

            NavIcon {
                anchors.centerIn: parent
                visible: row.isFolder
                implicitWidth: 12
                implicitHeight: 12
                name: row.expanded ? "chevron-down" : "chevron-right"
                color: row.theme.textMuted
            }

            MouseArea {
                anchors.fill: parent
                anchors.margins: -6
                enabled: row.isFolder
                onClicked: row.expandRequested(row.index)
            }
        }

        Rectangle {
            Layout.alignment: Qt.AlignVCenter
            implicitWidth: 16
            implicitHeight: 16
            radius: 3
            color: row.nodeState === Qt.Unchecked ? "transparent" : row.theme.accent
            border.width: 1
            border.color: row.nodeState === Qt.Unchecked
                          ? row.theme.hairline
                          : row.theme.accent
            opacity: row.available ? 1 : 0.45

            NavIcon {
                anchors.centerIn: parent
                visible: row.nodeState === Qt.Checked
                implicitWidth: 11
                implicitHeight: 11
                name: "check"
                color: row.theme.accentForeground
            }

            Rectangle {
                anchors.centerIn: parent
                visible: row.nodeState === Qt.PartiallyChecked
                width: 8
                height: 2
                radius: 1
                color: row.theme.accentForeground
            }
        }

        FileIcon {
            Layout.alignment: Qt.AlignVCenter
            implicitWidth: 16
            implicitHeight: 16
            kind: row.isFolder ? "folder" : "file"
            fileName: row.isFolder ? "" : row.nodeName
            color: row.isFolder ? row.theme.textSecondary : row.theme.textMuted
        }

        ColumnLayout {
            Layout.fillWidth: true
            spacing: 1

            ElidedLabel {
                Layout.fillWidth: true
                text: row.nodeName
                elide: Text.ElideMiddle
                color: row.theme.textPrimary
                font.pixelSize: row.theme.fontBody
                font.weight: row.isFolder ? Font.DemiBold : Font.Normal
            }

            Label {
                objectName: "magnetFileNote"
                Layout.fillWidth: true
                visible: row.nodeNote.length > 0
                text: row.nodeNote
                color: row.theme.warning
                font.pixelSize: row.theme.fontCaption
            }
        }

        Label {
            Layout.alignment: Qt.AlignVCenter
            visible: !row.isFolder
            text: row.nodeSize
            color: row.theme.textMuted
            font.pixelSize: row.theme.fontMeta
        }
    }
}
