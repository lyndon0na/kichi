import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 任务页顶部的分段切换：本机传输 / 离线任务。
// 只切换视图，不合并两边的数据、操作或轮询。
RowLayout {
    id: root

    property string currentTab: "transfers"
    property int transferCount: 0
    property int offlineCount: 0

    signal tabRequested(string tab)

    readonly property KTheme theme: KTheme { parent: root }

    spacing: theme.gapSm

    Rectangle {
        id: segment

        Layout.alignment: Qt.AlignVCenter
        implicitWidth: segmentRow.implicitWidth + 4
        implicitHeight: 32
        radius: theme.radiusControl
        color: theme.surfaceRaised
        border.width: 1
        border.color: theme.hairline

        Rectangle {
            id: selectionIndicator

            objectName: "taskTabSelectionIndicator"
            readonly property Item targetTab: root.currentTab === "offline"
                                                ? offlineTab : transfersTab
            x: segmentRow.x + (targetTab ? targetTab.x : 0)
            y: segmentRow.y
            width: targetTab ? targetTab.width : 0
            height: segmentRow.height
            radius: theme.radiusSmall
            color: theme.accentSoft
            visible: targetTab !== null

            Behavior on x {
                NumberAnimation {
                    duration: theme.motionFast
                    easing.type: Easing.OutCubic
                }
            }

            Behavior on width {
                NumberAnimation {
                    duration: theme.motionFast
                    easing.type: Easing.OutCubic
                }
            }
        }

        RowLayout {
            id: segmentRow

            anchors.fill: parent
            anchors.margins: 2
            spacing: 2

            Button {
                id: transfersTab

                objectName: "taskTabTransfers"
                readonly property bool selected: root.currentTab === "transfers"

                Layout.fillHeight: true
                implicitWidth: transfersContent.implicitWidth + 20
                padding: 0
                hoverEnabled: true
                focusPolicy: Qt.StrongFocus
                text: qsTr("本机传输")
                Accessible.name: text
                Accessible.description: root.transferCount > 0
                                        ? qsTr("%1 个进行中").arg(root.transferCount)
                                        : qsTr("没有未结束任务")
                onClicked: root.tabRequested("transfers")

                background: Rectangle {
                    radius: theme.radiusSmall
                    color: transfersTab.down ? theme.surfacePressed
                         : transfersTab.hovered
                           ? (transfersTab.selected ? theme.accentGhost
                                                    : theme.surfaceHover)
                         : theme.surfaceClear
                }

                contentItem: RowLayout {
                    id: transfersContent

                    spacing: 6

                    Item { Layout.fillWidth: true }

                    Label {
                        objectName: "taskTabTransfersLabel"
                        Layout.alignment: Qt.AlignVCenter
                        text: transfersTab.text
                        color: transfersTab.selected ? theme.accent : theme.textSecondary
                        font.pixelSize: theme.fontBody
                        font.weight: transfersTab.selected ? Font.DemiBold : Font.Normal
                    }

                    KStatusBadge {
                        objectName: "taskTabTransfersCount"
                        Layout.alignment: Qt.AlignVCenter
                        visible: root.transferCount > 0
                        text: root.transferCount > 99 ? "99+" : String(root.transferCount)
                        tone: transfersTab.selected ? "accent" : "neutral"
                    }

                    Item { Layout.fillWidth: true }
                }
            }

            Button {
                id: offlineTab

                objectName: "taskTabOffline"
                readonly property bool selected: root.currentTab === "offline"

                Layout.fillHeight: true
                implicitWidth: offlineContent.implicitWidth + 20
                padding: 0
                hoverEnabled: true
                focusPolicy: Qt.StrongFocus
                text: qsTr("离线任务")
                Accessible.name: text
                Accessible.description: root.offlineCount > 0
                                        ? qsTr("%1 条未结束").arg(root.offlineCount)
                                        : qsTr("没有未结束任务")
                onClicked: root.tabRequested("offline")

                background: Rectangle {
                    radius: theme.radiusSmall
                    color: offlineTab.down ? theme.surfacePressed
                         : offlineTab.hovered
                           ? (offlineTab.selected ? theme.accentGhost
                                                  : theme.surfaceHover)
                         : theme.surfaceClear
                }

                contentItem: RowLayout {
                    id: offlineContent

                    spacing: 6

                    Item { Layout.fillWidth: true }

                    Label {
                        objectName: "taskTabOfflineLabel"
                        Layout.alignment: Qt.AlignVCenter
                        text: offlineTab.text
                        color: offlineTab.selected ? theme.accent : theme.textSecondary
                        font.pixelSize: theme.fontBody
                        font.weight: offlineTab.selected ? Font.DemiBold : Font.Normal
                    }

                    KStatusBadge {
                        objectName: "taskTabOfflineCount"
                        Layout.alignment: Qt.AlignVCenter
                        visible: root.offlineCount > 0
                        text: root.offlineCount > 99 ? "99+" : String(root.offlineCount)
                        tone: offlineTab.selected ? "accent" : "neutral"
                    }

                    Item { Layout.fillWidth: true }
                }
            }
        }
    }

    Item {
        Layout.fillWidth: true
    }
}
