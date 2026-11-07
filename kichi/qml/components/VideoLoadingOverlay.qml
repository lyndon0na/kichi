import QtQuick
import QtQuick.Controls
import "../core"

// 视频取链 / 等待播放器窗口时的缩略图状态。
//
// 选定稿只保留一个视觉重心：压暗缩略图，在正中央悬浮加载环。
// 列表里整个缩略图都是取消热区；播放窗口里只作加载提示（cancellable: false），
// 由外部入口取消，可在加载环下方显示具体条目（label）。
Item {
    id: root

    property bool compact: false
    property int cornerRadius: 3
    /// 是否允许点击取消。播放窗口等外部已有取消入口时关掉。
    property bool cancellable: true
    /// 加载环下方的提示文字（为空时不占位）。
    property string label: ""
    signal cancelRequested()

    readonly property KTheme theme: KTheme { parent: root }

    Button {
        id: cancelButton
        objectName: "videoLoadingCancel"

        anchors.fill: parent
        enabled: root.cancellable
        hoverEnabled: root.cancellable
        padding: 0
        Accessible.name: root.label.length > 0
                         ? root.label
                         : (root.cancellable ? qsTr("取消视频加载")
                                             : qsTr("正在加载视频"))

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
            Column {
                anchors.centerIn: parent
                width: Math.max(0, Math.min(parent.width - 48, 560))
                spacing: 10

                KLoadingSpinner {
                    objectName: "videoLoadingSpinner"

                    anchors.horizontalCenter: parent.horizontalCenter
                    width: root.compact ? 14 : 28
                    height: width
                    running: root.visible && cancelButton.visible
                    color: root.theme.mediaForeground
                }

                Label {
                    objectName: "videoLoadingLabel"

                    visible: root.label.length > 0
                    width: parent.width
                    horizontalAlignment: Text.AlignHCenter
                    elide: Text.ElideMiddle
                    textFormat: Text.PlainText
                    text: root.label
                    color: root.theme.mediaForeground
                    font.pixelSize: root.compact ? root.theme.fontCaption
                                                 : root.theme.fontBody
                }
            }
        }

        ToolTip.visible: root.cancellable && hovered
        ToolTip.text: qsTr("取消这次视频加载")
        onClicked: {
            if (root.cancellable)
                root.cancelRequested()
        }
    }
}
