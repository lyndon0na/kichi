import QtQuick
import QtQuick.Controls

// 视频取链 / 等待播放器窗口时的缩略图状态。
//
// 选定稿只保留一个视觉重心：压暗缩略图，在正中央悬浮加载环。
// 整个缩略图都是取消热区；悬停提示说明点击动作，不再用底部状态条抢画面。
Item {
    id: root

    property bool compact: false
    property int cornerRadius: 3
    signal cancelRequested()

    readonly property KTheme theme: KTheme { parent: root }

    Button {
        id: cancelButton
        objectName: "videoLoadingCancel"

        anchors.fill: parent
        hoverEnabled: true
        padding: 0
        Accessible.name: qsTr("取消视频加载")

        background: Rectangle {
            objectName: "videoLoadingScrim"

            radius: root.cornerRadius
            color: root.compact ? root.theme.mediaScrimStrong : root.theme.mediaScrim

            Rectangle {
                anchors.fill: parent
                radius: parent.radius
                color: cancelButton.down
                       ? Qt.rgba(root.theme.mediaForeground.r,
                                 root.theme.mediaForeground.g,
                                 root.theme.mediaForeground.b, 0.16)
                       : cancelButton.hovered
                         ? Qt.rgba(root.theme.mediaForeground.r,
                                   root.theme.mediaForeground.g,
                                   root.theme.mediaForeground.b, 0.08)
                         : "transparent"
            }
        }

        contentItem: Item {
            KLoadingSpinner {
                objectName: "videoLoadingSpinner"

                anchors.centerIn: parent
                width: root.compact ? 14 : 28
                height: width
                running: root.visible && cancelButton.visible
                color: root.theme.mediaForeground
            }
        }

        ToolTip.visible: hovered
        ToolTip.text: qsTr("取消这次视频加载")
        onClicked: root.cancelRequested()
    }
}
