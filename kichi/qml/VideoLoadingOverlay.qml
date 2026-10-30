import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 视频取链 / 等待播放器窗口时的缩略图状态。
//
// 网格缩略图用底部状态条，保留画面主体并把取消入口放在条内；
// 列表缩略图只有 36×24，退化成整块可点的转圈层，点击即取消。
// 根 Item 本身不接收鼠标，只有状态条 / 紧凑按钮覆盖的控件会吃掉点击。
Item {
    id: root

    property bool compact: false
    property int cornerRadius: 3
    signal cancelRequested()

    readonly property KTheme theme: KTheme { parent: root }

    Rectangle {
        id: strip
        objectName: "videoLoadingStrip"

        visible: !root.compact
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        height: 34
        topLeftRadius: 0
        topRightRadius: 0
        bottomLeftRadius: root.cornerRadius
        bottomRightRadius: root.cornerRadius
        color: root.theme.mediaBadge

        Rectangle {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            height: 1
            color: Qt.rgba(root.theme.mediaForeground.r,
                           root.theme.mediaForeground.g,
                           root.theme.mediaForeground.b, 0.13)
        }

        RowLayout {
            anchors.fill: parent
            anchors.leftMargin: 9
            anchors.rightMargin: 5
            anchors.topMargin: 4
            anchors.bottomMargin: 4
            spacing: 7

            KLoadingSpinner {
                objectName: "videoLoadingSpinner"

                running: root.visible && strip.visible
                color: root.theme.mediaForeground
                Layout.preferredWidth: 16
                Layout.preferredHeight: 16
                Layout.alignment: Qt.AlignVCenter
            }

            ElidedLabel {
                objectName: "videoLoadingText"

                Layout.fillWidth: true
                Layout.alignment: Qt.AlignVCenter
                text: qsTr("正在加载…")
                font.pixelSize: root.theme.fontCaption
                color: root.theme.mediaForeground
                elide: Text.ElideRight
                toolTipEnabled: false
            }

            Button {
                id: cancelButton
                objectName: "videoLoadingCancel"

                Layout.preferredHeight: 24
                Layout.alignment: Qt.AlignVCenter
                leftPadding: 7
                rightPadding: 7
                topPadding: 3
                bottomPadding: 3
                hoverEnabled: true
                text: qsTr("取消")
                Accessible.name: qsTr("取消视频加载")

                background: Rectangle {
                    radius: root.theme.radiusSmall
                    color: cancelButton.down
                           ? Qt.rgba(root.theme.mediaForeground.r,
                                     root.theme.mediaForeground.g,
                                     root.theme.mediaForeground.b, 0.24)
                           : cancelButton.hovered
                             ? Qt.rgba(root.theme.mediaForeground.r,
                                       root.theme.mediaForeground.g,
                                       root.theme.mediaForeground.b, 0.14)
                             : "transparent"
                }

                contentItem: Text {
                    objectName: "videoLoadingCancelText"

                    text: cancelButton.text
                    font.family: cancelButton.font.family
                    font.pixelSize: root.theme.fontCaption
                    font.weight: Font.DemiBold
                    color: root.theme.mediaForeground
                    horizontalAlignment: Text.AlignHCenter
                    verticalAlignment: Text.AlignVCenter
                }

                ToolTip.visible: hovered
                ToolTip.text: qsTr("取消这次视频加载")
                onClicked: root.cancelRequested()
            }
        }
    }

    Button {
        id: compactButton
        objectName: "videoLoadingCompactCancel"

        visible: root.compact
        anchors.fill: parent
        Accessible.name: qsTr("取消视频加载")

        background: Rectangle {
            radius: root.cornerRadius
            color: compactButton.down
                   ? Qt.rgba(root.theme.mediaForeground.r,
                             root.theme.mediaForeground.g,
                             root.theme.mediaForeground.b, 0.24)
                   : compactButton.hovered
                     ? Qt.rgba(root.theme.mediaForeground.r,
                               root.theme.mediaForeground.g,
                               root.theme.mediaForeground.b, 0.14)
                     : root.theme.mediaBadge
        }

        contentItem: KLoadingSpinner {
            running: root.visible && compactButton.visible
            color: root.theme.mediaForeground
        }

        ToolTip.visible: hovered
        ToolTip.text: qsTr("取消这次视频加载")
        onClicked: root.cancelRequested()
    }
}
