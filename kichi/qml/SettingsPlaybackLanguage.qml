import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 设置页「播放」段的语言偏好：只影响内嵌播放器，下次开播生效。
ColumnLayout {
    id: languages

    property var backend: null

    spacing: 0

    KSettingsRow {
        title: qsTr("首选音轨语言")
        description: qsTr("内嵌播放时优先选择该语言的音轨；下次开播生效，没有时跟随文件默认。")

        KComboBox {
            id: audioLanguageBox
            objectName: "settingsAudioLanguage"
            width: 240
            model: [
                { "value": "auto", "label": qsTr("自动（跟随系统语言）") },
                { "value": "zh", "label": qsTr("中文") },
                { "value": "yue", "label": qsTr("粤语") },
                { "value": "en", "label": qsTr("英语") },
                { "value": "ja", "label": qsTr("日语") },
                { "value": "ko", "label": qsTr("韩语") }
            ]
            textRole: "label"
            valueRole: "value"

            Component.onCompleted: languages.syncFromBackend()
            function syncFromBackend() {
                if (!languages.backend)
                    return
                var index = audioLanguageBox.indexOfValue(languages.backend.playerAudioLang)
                if (index >= 0 && index !== audioLanguageBox.currentIndex)
                    audioLanguageBox.currentIndex = index
            }
            onActivated: function (index) {
                if (!languages.backend)
                    return
                languages.backend.setPlayerAudioLang(audioLanguageBox.model[index].value)
            }
        }
    }

    KSettingsRow {
        title: qsTr("首选字幕语言")
        description: qsTr("内嵌播放时优先选择该语言的内封 / 外挂字幕；下次开播生效，没有时回退中文优先。")

        KComboBox {
            id: subtitleLanguageBox
            objectName: "settingsSubtitleLanguage"
            width: 240
            model: [
                { "value": "auto", "label": qsTr("自动（跟随系统语言）") },
                { "value": "zh", "label": qsTr("中文") },
                { "value": "yue", "label": qsTr("粤语") },
                { "value": "en", "label": qsTr("英语") },
                { "value": "ja", "label": qsTr("日语") },
                { "value": "ko", "label": qsTr("韩语") }
            ]
            textRole: "label"
            valueRole: "value"

            Component.onCompleted: languages.syncFromBackend()
            function syncFromBackend() {
                if (!languages.backend)
                    return
                var index = subtitleLanguageBox.indexOfValue(languages.backend.playerSubLang)
                if (index >= 0 && index !== subtitleLanguageBox.currentIndex)
                    subtitleLanguageBox.currentIndex = index
            }
            onActivated: function (index) {
                if (!languages.backend)
                    return
                languages.backend.setPlayerSubLang(subtitleLanguageBox.model[index].value)
            }
        }
    }

    Connections {
        target: languages.backend
        function onChanged() {
            languages.syncFromBackend()
        }
    }

    function syncFromBackend() {
        audioLanguageBox.syncFromBackend()
        subtitleLanguageBox.syncFromBackend()
    }
}
