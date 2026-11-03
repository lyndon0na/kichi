import QtQuick
import QtTest
import ".."

// 回归：KTheme 挂在属性上可以避免影响默认内容项识别，但对象如果没有父项，
// palette 会停在创建时的值，运行中切换 Plasma 配色后设置页不会实时刷新。
TestCase {
    name: "ThemeLivePalette"
    when: windowShown
    width: 800
    height: 800

    Item {
        id: host
        width: 800
        height: 800

        function setGroupBase(value) {
            group.palette.base = value
        }

        function setDialogWindow(value) {
            dialog.palette.window = value
        }

        readonly property KTheme groupTheme: group.theme
        readonly property color groupSurface: group.theme.surfaceRaised
        readonly property KTheme dialogTheme: dialog.theme
        readonly property color dialogSurface: dialog.theme.surfaceSheet

        KSettingsGroup {
            id: group
            width: 400
            title: "test"
        }

        KSettingsRow {
            id: row
            width: 400
            title: "test"
        }

        SettingsAbout {
            id: about
            width: 400
        }

        SettingsPlayback {
            id: playback
            width: 400
        }

        KDialog {
            id: dialog
            parent: host
        }

        KButton {
            id: dangerButton
            objectName: "themeDangerButton"
            width: 140
            text: "危险操作"
            variant: "danger"
        }

        KButton {
            id: primaryButton
            objectName: "themePrimaryButton"
            width: 140
            text: "主操作"
            variant: "primary"
        }
    }

    function useLightPalette() {
        host.palette.base = "#ffffff"
        host.palette.window = "#f7f7f7"
        host.palette.text = "#000000"
        host.palette.highlight = "#3daee9"
        host.palette.highlightedText = "#ffffff"
        wait(0)
    }

    function test_group_theme_follows_live_palette() {
        var before = String(host.groupSurface)
        host.setGroupBase("#123456")
        wait(0)
        compare(String(host.groupTheme.palette.base), "#123456")
        var after = String(host.groupSurface)
        verify(after !== before, "surfaceRaised 必须随 palette.base 实时更新（"
               + before + " -> " + after + "，base " + host.groupTheme.palette.base + "）")
    }

    function test_theme_objects_are_parented() {
        verify(group.theme.parent === group)
        verify(row.theme.parent === row)
        verify(about.theme.parent === about)
        verify(playback.theme.parent === playback)
        verify(dialog.theme.parent === dialog.background)
    }

    function test_group_still_has_layout_height() {
        verify(group.height > 0, "不可见主题对象不能弄塌分组布局")
    }

    function test_dialog_theme_follows_live_palette() {
        var before = String(host.dialogSurface)
        host.setDialogWindow("#345678")
        wait(0)
        compare(String(host.dialogTheme.palette.window), "#345678")
        var after = String(host.dialogSurface)
        verify(after !== before, "弹窗表面色必须随宿主窗口 palette 实时更新（"
               + before + " -> " + after + "）")
    }

    function test_solid_button_foreground_uses_theme_tokens() {
        useLightPalette()
        compare(String(dangerButton.background.color), "#c0392b")
        compare(String(dangerButton.foregroundColor), "#ffffff")
        compare(String(primaryButton.foregroundColor), "#ffffff")

        var dangerLabel = findChild(dangerButton, "buttonText")
        verify(dangerLabel !== null)
        compare(String(dangerLabel.color), "#ffffff")
    }
}
