import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 设置页「播放」段：默认清晰度 + 播放器命令 / 参数。
// 从 SettingsPage 拆出来只为守 QML 行数提醒线，行为没有变化。
ColumnLayout {
    id: playback

    property var backend: null

    // 输入框在编辑中时不要被 backend 的刷新覆盖（否则打字会被打断）
    property bool commandDirty: false
    property bool argsDirty: false

    spacing: 16

    Label {
        text: qsTr("播放")
        font.bold: true
    }

    RowLayout {
        Layout.fillWidth: true
        spacing: 12

        Label {
            text: qsTr("默认清晰度")
            Layout.preferredWidth: 110
        }

        ComboBox {
            id: qualityBox
            Layout.preferredWidth: 220
            model: [
                { "value": "ask", "label": qsTr("每次询问（弹出档位菜单）") },
                { "value": "original", "label": qsTr("优先原画") },
                { "value": "transcode", "label": qsTr("优先最高转码档") }
            ]
            textRole: "label"
            valueRole: "value"

            Component.onCompleted: syncFromBackend()
            Connections {
                target: playback.backend
                function onChanged() {
                    qualityBox.syncFromBackend()
                }
            }
            function syncFromBackend() {
                if (!playback.backend)
                    return
                var index = qualityBox.indexOfValue(playback.backend.preferredQuality)
                if (index >= 0 && index !== qualityBox.currentIndex)
                    qualityBox.currentIndex = index
            }
            // onActivated 时不保证 currentValue 已更新，用 index 从 model 里取
            onActivated: function (index) {
                if (!playback.backend)
                    return
                playback.backend.setPreferredQuality(qualityBox.model[index].value)
            }
        }

        Label {
            Layout.fillWidth: true
            wrapMode: Text.Wrap
            color: palette.placeholderText
            font.pixelSize: 12
            text: qsTr("「每次询问」总是弹档位菜单；选「优先」则直接起播。只有原画一档的文件，选转码也会直接播原画。")
        }
    }

    GridLayout {
        Layout.fillWidth: true
        columns: 3
        columnSpacing: 12
        rowSpacing: 8

        Label {
            text: qsTr("播放器命令")
            Layout.preferredWidth: 110
        }
        TextField {
            id: commandField
            Layout.fillWidth: true
            selectByMouse: true
            placeholderText: playback.backend ? playback.backend.playerDefaultCommand : ""
            text: playback.backend ? playback.backend.playerCommand : ""
            onTextEdited: playback.commandDirty = true
            onEditingFinished: playback.applyPlayer()
        }
        Label { text: "" }

        Label {
            text: qsTr("附加参数")
            Layout.preferredWidth: 110
        }
        TextField {
            id: argsField
            Layout.fillWidth: true
            selectByMouse: true
            placeholderText: playback.backend ? playback.backend.playerDefaultArgs : ""
            text: playback.backend ? playback.backend.playerArgs : ""
            onTextEdited: playback.argsDirty = true
            onEditingFinished: playback.applyPlayer()
        }
        RowLayout {
            spacing: 6

            Button {
                text: qsTr("应用")
                enabled: playback.commandDirty || playback.argsDirty
                onClicked: playback.applyPlayer()
            }
            Button {
                text: qsTr("恢复默认")
                flat: true
                onClicked: {
                    playback.commandDirty = false
                    playback.argsDirty = false
                    if (playback.backend)
                        playback.backend.resetPlayer()
                }
            }
        }
    }

    Label {
        Layout.fillWidth: true
        wrapMode: Text.Wrap
        color: palette.placeholderText
        font.pixelSize: 12
        text: qsTr("默认：%1 %2。留空表示使用默认；文件名用 {title} 占位，直链走临时清单不进命令行。")
                .arg(playback.backend ? playback.backend.playerDefaultCommand : "")
                .arg(playback.backend ? playback.backend.playerDefaultArgs : "")
    }

    function applyPlayer() {
        if (!playback.backend)
            return
        playback.commandDirty = false
        playback.argsDirty = false
        playback.backend.setPlayer(commandField.text, argsField.text)
    }
}
