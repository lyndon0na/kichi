import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "../components"
import "../core"

// 设置对话框左侧大纲：只列分组，点选后右侧整块换页，不做滚动跟随。
// 卡片式浮层里大纲不铺底色、不画竖分隔线，靠行距和选中药丸分区；
// 行高比主侧栏略高，和图标的 18px 视觉重量对齐。
Item {
    id: outline

    /// [{ label, icon }]，与 SettingsPage 的 sections 一一对应
    property var sections: []
    property int currentIndex: 0
    /// 当前选中行；选中药丸跟着它换位
    property Item selectedItem: null

    signal sectionRequested(int index)

    KTheme { id: theme }

    // 全大纲只有一个选中药丸，切组时滑动到新行（和主侧栏同一思路）。
    // 画在行下面：行自身的底色只负责悬停，不参与选中态。
    Rectangle {
        id: selectionPill

        objectName: "settingsSelectionPill"
        visible: outline.selectedItem !== null
        enabled: false
        x: 20
        // 行在大纲里的位置 = 行自身 y + 所在列布局的 y。
        // 不用 mapToItem：那是一次性求值，列布局稍后应用锚点边距时不会再算。
        y: outline.selectedItem && outline.selectedItem.parent
           ? outline.selectedItem.y + outline.selectedItem.parent.y
           : 0
        width: Math.max(0, outline.width - 20 - 12)
        height: outline.selectedItem ? outline.selectedItem.height : 0
        radius: theme.radiusSurface
        color: theme.accentSoft
        border.width: 1
        border.color: Qt.rgba(theme.accent.r, theme.accent.g, theme.accent.b, 0.28)

        Behavior on y {
            NumberAnimation {
                duration: theme.motionNormal
                easing.type: Easing.OutCubic
            }
        }
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.leftMargin: 20
        anchors.rightMargin: 12
        anchors.topMargin: 2
        anchors.bottomMargin: theme.gapLg
        spacing: 2

        Repeater {
            model: outline.sections

            delegate: Button {
                id: sectionButton

                required property var modelData
                required property int index

                readonly property bool selected: outline.currentIndex === index

                objectName: "settingsOutlineItem_" + index
                Layout.fillWidth: true
                Layout.preferredHeight: 42
                padding: 0
                leftPadding: 12
                rightPadding: 12

                background: Rectangle {
                    radius: theme.radiusSurface
                    color: sectionButton.hovered ? theme.accentGhost
                                                 : theme.accentClear
                }

                onSelectedChanged: {
                    if (sectionButton.selected)
                        outline.selectedItem = sectionButton
                }

                Component.onCompleted: {
                    if (sectionButton.selected)
                        outline.selectedItem = sectionButton
                }

                contentItem: RowLayout {
                    spacing: theme.gapMd

                    NavIcon {
                        implicitWidth: 18
                        implicitHeight: 18
                        name: sectionButton.modelData.icon
                        color: sectionButton.selected ? theme.accent : theme.textMuted
                        Layout.alignment: Qt.AlignVCenter
                    }

                    ElidedLabel {
                        Layout.fillWidth: true
                        text: sectionButton.modelData.label
                        font.pixelSize: theme.fontSection
                        font.weight: sectionButton.selected ? Font.DemiBold : Font.Normal
                        color: theme.textPrimary
                        elide: Text.ElideRight
                    }
                }

                Accessible.name: sectionButton.modelData.label
                onClicked: outline.sectionRequested(index)
            }
        }

        Item { Layout.fillHeight: true }

        RowLayout {
            Layout.fillWidth: true
            Layout.leftMargin: 12
            spacing: theme.gapXs

            NavIcon {
                implicitWidth: 12
                implicitHeight: 12
                name: "check"
                color: theme.textMuted
                opacity: 0.8
                Layout.alignment: Qt.AlignVCenter
            }

            Label {
                text: qsTr("更改会自动保存")
                font.pixelSize: theme.fontCaption
                color: theme.textMuted
            }
        }
    }
}
