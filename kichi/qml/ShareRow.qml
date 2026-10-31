import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 分享管理表格的一行：文件图标、名称、状态、有效期和创建时间。
// 行内“复制链接”同时复制访问密码；整行支持 Ctrl / Shift 多选和右键菜单。
ItemDelegate {
    id: row

    required property string key
    required property string title
    required property string kind
    required property string url
    required property string copyText
    required property string expirationText
    required property string createdText
    required property string statsText
    required property string statusText
    required property string statusTone
    required property int index

    property int rowHeight: 48
    property int iconColumnWidth: 22
    property int statusColumnWidth: 100
    property int expirationColumnWidth: 64
    property int createdColumnWidth: 120
    property int actionColumnWidth: 96
    property int columnSpacing: 12
    property int contentPadding: 12
    // 列表视图本身；附加属性只能在委托根上读取。
    readonly property var listView: ListView.view
    /// 在列表里的下标（Shift 区间选择用）
    property int rowIndex: index

    signal copyRequested(string text)
    signal selectRequested(int modifiers, int index)
    signal contextMenu(point position)

    KTheme { id: theme }

    implicitHeight: row.rowHeight
    leftPadding: row.contentPadding
    rightPadding: row.contentPadding
    topPadding: 0
    bottomPadding: 0
    hoverEnabled: true

    // 关掉 ItemDelegate 自带图标；文件类型图标在内容区单独渲染。
    icon.name: ""
    icon.source: ""
    icon.width: 0
    icon.height: 0
    icon.color: "transparent"

    background: Rectangle {
        // MouseArea 位于内容层下方，不能可靠接收整行 hover；
        // ItemDelegate 自己的 hovered 不受子项遮挡影响。
        color: row.highlighted
               ? theme.accentSoft
               : mouseArea.pressed
                 ? theme.surfacePressed
                 : row.hovered
                   ? theme.surfaceHover
                   : theme.surfaceClear

        Rectangle {
            visible: row.highlighted
            width: 3
            height: Math.max(0, parent.height - 28)
            radius: 1.5
            anchors.left: parent.left
            anchors.leftMargin: 1
            anchors.verticalCenter: parent.verticalCenter
            color: theme.accent
        }

        Rectangle {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: 1
            color: theme.divider
        }
    }

    contentItem: Item {
        // MouseArea 放在内容布局下面，既接住整行的左右键，又不挡住“复制链接”按钮。
        MouseArea {
            id: mouseArea
            anchors.fill: parent
            z: 0
            acceptedButtons: Qt.LeftButton | Qt.RightButton
            hoverEnabled: true

            onClicked: function (mouse) {
                if (row.listView)
                    row.listView.forceActiveFocus()
                if (mouse.button === Qt.RightButton) {
                    row.contextMenu(Qt.point(mouse.x, mouse.y))
                    return
                }
                row.selectRequested(mouse.modifiers, row.rowIndex)
            }
        }

        RowLayout {
            id: rowLayout
            anchors.fill: parent
            z: 1
            spacing: row.columnSpacing

            Item {
                Layout.preferredWidth: row.iconColumnWidth
                Layout.minimumWidth: row.iconColumnWidth
                Layout.fillHeight: true

                FileIcon {
                    objectName: "shareTypeIcon"
                    width: 18
                    height: 18
                    anchors.centerIn: parent
                    kind: row.kind
                    fileName: row.title
                    color: theme.textSecondary
                }
            }

            ColumnLayout {
                Layout.fillWidth: true
                Layout.fillHeight: true
                spacing: 2

                ElidedLabel {
                    Layout.fillWidth: true
                    text: row.title
                    color: theme.textPrimary
                    font.pixelSize: theme.fontBody
                    font.weight: Font.DemiBold
                    elide: Text.ElideRight
                    verticalAlignment: Text.AlignVCenter
                }

                ElidedLabel {
                    Layout.fillWidth: true
                    text: row.statsText
                    color: theme.textMuted
                    font.pixelSize: theme.fontCaption
                    elide: Text.ElideRight
                    verticalAlignment: Text.AlignVCenter
                }
            }

            Item {
                Layout.preferredWidth: row.statusColumnWidth
                Layout.minimumWidth: row.statusColumnWidth
                Layout.fillHeight: true

                KStatusBadge {
                    anchors.centerIn: parent
                    text: row.statusText
                    tone: row.statusTone
                }
            }

            Item {
                Layout.preferredWidth: row.expirationColumnWidth
                Layout.minimumWidth: row.expirationColumnWidth
                Layout.fillHeight: true

                ElidedLabel {
                    anchors.fill: parent
                    text: row.expirationText
                    color: theme.textSecondary
                    font.pixelSize: theme.fontMeta
                    horizontalAlignment: Text.AlignHCenter
                    verticalAlignment: Text.AlignVCenter
                    elide: Text.ElideRight
                }
            }

            Item {
                Layout.preferredWidth: row.createdColumnWidth
                Layout.minimumWidth: row.createdColumnWidth
                Layout.fillHeight: true

                ElidedLabel {
                    anchors.fill: parent
                    text: row.createdText
                    color: theme.textSecondary
                    font.pixelSize: theme.fontMeta
                    horizontalAlignment: Text.AlignRight
                    verticalAlignment: Text.AlignVCenter
                    elide: Text.ElideRight
                }
            }

            KButton {
                objectName: "shareCopyButton"
                Layout.preferredWidth: row.actionColumnWidth
                Layout.minimumWidth: row.actionColumnWidth
                implicitHeight: 30
                text: qsTr("复制链接")
                variant: "ghost"
                enabled: row.url.length > 0
                Accessible.name: qsTr("复制分享「%1」的链接和密码").arg(row.title)
                onClicked: row.copyRequested(row.copyText)
            }
        }
    }
}
