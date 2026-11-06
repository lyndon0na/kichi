import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 播放窗口的倍速面板：0.5×–3× 滑块，拖动实时改 mpv 倍速。
//
// 值仍由 PlaybackWindow.speedPercent 统一持有；这里只发预览 / 提交信号，
// 不在内部存副本。松手或关闭面板时提交持久化，避免拖动过程反复写设置文件。
Rectangle {
    id: panel

    property var backend: null
    property bool open: false
    property int current: 100
    /// 底栏：面板贴它上方，高度不从外部 id 猜
    property Item anchorItem: null

    signal speedEdited(int percent)
    signal speedCommitted(int percent)

    readonly property KTheme theme: KTheme { parent: panel }
    readonly property int speedStep: 5

    anchors.right: parent.right
    anchors.rightMargin: theme.gapLg
    anchors.bottom: anchorItem ? anchorItem.top : parent.bottom
    anchors.bottomMargin: theme.gapSm
    width: 300
    height: 106
    radius: theme.mediaRadius
    color: theme.mediaSurface
    border.width: 1
    border.color: theme.mediaBorder
    opacity: open ? 1 : 0
    visible: opacity > 0
    Behavior on opacity { NumberAnimation { duration: theme.motionFast } }

    function formatSpeed(percent) {
        return (Math.round(percent) / 100) + "×"
    }

    function setSpeed(percent) {
        percent = Math.max(50, Math.min(300, Math.round(percent)))
        speedSlider.value = percent
        speedEdited(percent)
        speedCommitted(percent)
    }

    onOpenChanged: {
        if (open) {
            speedSlider.value = panel.current
        } else if (panel.backend !== null && panel.backend.embedActive === true) {
            speedCommitted(Math.round(speedSlider.value))
        }
    }

    // mpv 原生键位（[ / ]）改过倍速时，面板开着也跟着走；拖动中不抢。
    onCurrentChanged: {
        if (!speedSlider.pressed)
            speedSlider.value = panel.current
    }

    // 吃掉面板上的点击，别让它们穿透到画面去切换控制条。
    MouseArea {
        anchors.fill: parent
        acceptedButtons: Qt.LeftButton
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: theme.gapMd
        spacing: theme.gapSm

        RowLayout {
            Layout.fillWidth: true
            spacing: theme.gapSm

            Label {
                text: qsTr("播放速度")
                color: theme.mediaText
                font.pixelSize: theme.fontSection
                font.weight: Font.Medium
            }

            Item { Layout.fillWidth: true }

            Rectangle {
                id: resetButton

                visible: panel.current !== 100
                implicitWidth: 46
                implicitHeight: 22
                radius: theme.radiusSmall
                color: resetMouse.containsMouse ? theme.mediaHover : "transparent"

                Label {
                    anchors.centerIn: parent
                    text: qsTr("回 1×")
                    color: theme.mediaTextMuted
                    font.pixelSize: theme.fontCaption
                }

                MouseArea {
                    id: resetMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: panel.setSpeed(100)
                }
            }

            Label {
                text: panel.formatSpeed(panel.current)
                color: theme.mediaText
                font.pixelSize: theme.fontSection
                font.weight: Font.Medium
            }
        }

        Slider {
            id: speedSlider

            Layout.fillWidth: true
            Layout.preferredHeight: 26
            from: 50
            to: 300
            stepSize: panel.speedStep
            snapMode: Slider.SnapAlways
            focusPolicy: Qt.NoFocus
            value: 100
            onMoved: panel.speedEdited(Math.round(value))
            onPressedChanged: {
                if (!pressed)
                    panel.speedCommitted(Math.round(value))
            }

            background: Rectangle {
                x: speedSlider.leftPadding
                y: speedSlider.topPadding + speedSlider.availableHeight / 2 - height / 2
                width: speedSlider.availableWidth
                height: 4
                radius: 2
                color: "#3DFFFFFF"

                Rectangle {
                    width: speedSlider.visualPosition * parent.width
                    height: parent.height
                    radius: 2
                    color: "#F2FFFFFF"
                }
            }

            handle: Rectangle {
                x: speedSlider.leftPadding
                   + speedSlider.visualPosition * (speedSlider.availableWidth - width)
                y: speedSlider.topPadding + speedSlider.availableHeight / 2 - height / 2
                width: 14
                height: 14
                radius: 7
                color: "#F2FFFFFF"
            }
        }

        Item {
            Layout.fillWidth: true
            Layout.preferredHeight: 14

            Repeater {
                model: [50, 100, 150, 200, 250, 300]

                delegate: Label {
                    text: (modelData / 100) + "×"
                    color: panel.current === modelData ? panel.theme.mediaText
                                                       : panel.theme.mediaTextMuted
                    font.pixelSize: panel.theme.fontCaption
                    x: Math.max(0, Math.min(parent.width - width,
                                           (modelData - 50) / 250 * parent.width - width / 2))
                }
            }
        }
    }
}
