import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "../components"
import "../core"

// 播放窗口的字幕样式面板：字号 / 位置 / 文字颜色 / 描边粗细 / 描边颜色。
//
// 每次调整都立刻写回后端（`setSubtitle*`），后端再下发给正在播放的 mpv，
// 所以画面里的字幕就是实时预览；面板里的预览行只是给一个大致参照。
// 值全部来自 backend 属性绑定，这里不存副本——设置页改动也能同步过来。
Rectangle {
    id: panel

    property var backend: null
    property bool open: false
    /// 底栏：面板贴它上方，高度不从外部 id 猜
    property Item anchorItem: null

    signal closeRequested()

    readonly property int fontSize: backend ? backend.embedSubFontSize : 55
    readonly property int position: backend ? backend.embedSubPos : 100
    readonly property string textColor: backend ? backend.embedSubColor : "#FFFFFFFF"
    readonly property int outlineSize: backend ? backend.embedSubOutlineSize : 2
    readonly property string outlineColor: backend ? backend.embedSubOutlineColor : "#FF000000"

    readonly property var textColors: [
        { "color": "#FFFFFFFF", "name": qsTr("白色"), "darkText": true },
        { "color": "#FFFFFF00", "name": qsTr("黄色"), "darkText": true },
        { "color": "#FF00FFFF", "name": qsTr("青色"), "darkText": true },
        { "color": "#FF9BE37F", "name": qsTr("绿色"), "darkText": true },
        { "color": "#FFFF9EC9", "name": qsTr("粉色"), "darkText": true },
        { "color": "#FFFFA94D", "name": qsTr("橙色"), "darkText": true }
    ]
    readonly property var outlineColors: [
        { "color": "#FF000000", "name": qsTr("黑色"), "darkText": false },
        { "color": "#FFFFFFFF", "name": qsTr("白色"), "darkText": true },
        { "color": "#FF404040", "name": qsTr("深灰"), "darkText": false },
        { "color": "#FF1A2A4A", "name": qsTr("深蓝"), "darkText": false },
        { "color": "#FF4A1A1A", "name": qsTr("深红"), "darkText": false }
    ]

    KTheme { id: theme }

    anchors.right: parent.right
    anchors.rightMargin: theme.gapLg
    anchors.bottom: anchorItem ? anchorItem.top : parent.bottom
    anchors.bottomMargin: theme.gapSm
    width: 320
    // 小窗口下给滚动留出空间，别把面板顶出画面。
    height: Math.min(body.implicitHeight + theme.gapLg * 2,
                     Math.max(180, parent.height - (anchorItem ? anchorItem.height : 0)
                              - theme.gapLg * 2))
    radius: theme.mediaRadius
    color: theme.mediaSurface
    border.width: 1
    border.color: theme.mediaBorder
    opacity: open ? 1 : 0
    visible: opacity > 0
    Behavior on opacity { NumberAnimation { duration: theme.motionFast } }

    // 吃掉面板上的点击，别让它们穿透到画面去切换控制条。
    MouseArea {
        anchors.fill: parent
        acceptedButtons: Qt.LeftButton
    }

    Flickable {
        id: flick

        anchors.fill: parent
        anchors.margins: theme.gapLg
        contentHeight: body.implicitHeight
        clip: true
        boundsBehavior: Flickable.StopAtBounds

        ColumnLayout {
            id: body

            width: flick.width
            spacing: theme.gapMd

            RowLayout {
                Layout.fillWidth: true
                spacing: theme.gapSm

                Label {
                    Layout.fillWidth: true
                    text: qsTr("字幕样式")
                    color: "white"
                    font.pixelSize: theme.fontSection
                    font.weight: Font.Medium
                }

                KIconButton {
                    objectName: "playbackSubtitlePanelClose"
                    iconName: "x"
                    iconSize: 14
                    iconColor: "white"
                    iconOpacity: 0.85
                    onMedia: true
                    implicitWidth: 26
                    implicitHeight: 26
                    focusPolicy: Qt.NoFocus
                    Accessible.name: qsTr("关闭字幕样式")
                    onClicked: panel.closeRequested()
                }
            }

            // 预览：颜色 / 字号 / 描边颜色的粗略参照；位置与描边宽度以画面为准。
            Rectangle {
                Layout.fillWidth: true
                implicitHeight: 46
                radius: theme.radiusSmall
                color: "#33000000"

                Label {
                    anchors.centerIn: parent
                    text: qsTr("字幕预览")
                    color: panel.textColor
                    font.pixelSize: Math.max(12, Math.min(24, Math.round(panel.fontSize * 0.42)))
                    style: Text.Outline
                    styleColor: panel.outlineColor
                }
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: theme.gapSm

                Label {
                    Layout.fillWidth: true
                    text: qsTr("字号")
                    color: "#CCFFFFFF"
                    font.pixelSize: theme.fontMeta
                }

                KStepper {
                    objectName: "playbackSubtitleFontSize"
                    value: panel.fontSize
                    from: 16
                    to: 100
                    accessibleName: qsTr("字幕字号")
                    onValueEdited: function (value) {
                        if (panel.backend)
                            panel.backend.setSubtitleFontSize(value)
                    }
                }
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: theme.gapSm

                Label {
                    Layout.fillWidth: true
                    text: qsTr("位置")
                    color: "#CCFFFFFF"
                    font.pixelSize: theme.fontMeta
                }

                KStepper {
                    objectName: "playbackSubtitlePosition"
                    value: panel.position
                    from: 0
                    to: 150
                    accessibleName: qsTr("字幕位置")
                    onValueEdited: function (value) {
                        if (panel.backend)
                            panel.backend.setSubtitlePosition(value)
                    }
                }
            }

            Label {
                Layout.fillWidth: true
                text: qsTr("0 = 贴顶部，100 = 底部默认位置，150 = 更靠下。")
                color: "#80FFFFFF"
                font.pixelSize: theme.fontCaption
                wrapMode: Text.WordWrap
            }

            ColumnLayout {
                Layout.fillWidth: true
                spacing: theme.gapXs

                Label {
                    text: qsTr("文字颜色")
                    color: "#CCFFFFFF"
                    font.pixelSize: theme.fontMeta
                }

                KColorSwatchRow {
                    objectName: "playbackSubtitleTextColor"
                    colors: panel.textColors
                    current: panel.textColor
                    onColorPicked: function (color) {
                        if (panel.backend)
                            panel.backend.setSubtitleColor(color)
                    }
                }
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: theme.gapSm

                Label {
                    Layout.fillWidth: true
                    text: qsTr("描边粗细")
                    color: "#CCFFFFFF"
                    font.pixelSize: theme.fontMeta
                }

                KStepper {
                    objectName: "playbackSubtitleOutlineSize"
                    value: panel.outlineSize
                    from: 0
                    to: 8
                    accessibleName: qsTr("字幕描边粗细")
                    onValueEdited: function (value) {
                        if (panel.backend)
                            panel.backend.setSubtitleOutlineSize(value)
                    }
                }
            }

            ColumnLayout {
                Layout.fillWidth: true
                spacing: theme.gapXs

                Label {
                    text: qsTr("描边颜色")
                    color: "#CCFFFFFF"
                    font.pixelSize: theme.fontMeta
                }

                KColorSwatchRow {
                    objectName: "playbackSubtitleOutlineColor"
                    colors: panel.outlineColors
                    current: panel.outlineColor
                    onColorPicked: function (color) {
                        if (panel.backend)
                            panel.backend.setSubtitleOutlineColor(color)
                    }
                }
            }
        }
    }
}
