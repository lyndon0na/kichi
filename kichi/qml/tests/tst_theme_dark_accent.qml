import QtQuick
import QtTest
import "../core"

// 回归：深色调色板 + 强调色必须完整落到 KTheme 令牌上。
// 非 KDE 桌面的系统外观由 Rust 侧注入 QGuiApplication palette，
// 这里直接改宿主 palette 模拟同一条路径。
TestCase {
    name: "ThemeDarkAccent"
    when: windowShown
    width: 400
    height: 400

    Item {
        id: host
        width: 400
        height: 400

        readonly property KTheme theme: KTheme { parent: host }
    }

    function useDarkPalette() {
        host.palette.base = "#1e1e1e"
        host.palette.window = "#242424"
        host.palette.text = "#f6f6f6"
        host.palette.placeholderText = "#8a8a8a"
        host.palette.highlight = "#3584e4"
        host.palette.highlightedText = "#ffffff"
        wait(0)
    }

    function test_dark_tokens_follow_palette() {
        useDarkPalette()

        verify(host.theme.darkMode, "深色底必须被识别为 darkMode")
        compare(String(host.theme.surface), "#1e1e1e")
        compare(String(host.theme.surfaceSheet), "#242424")
        compare(String(host.theme.textPrimary), "#f6f6f6")
        compare(String(host.theme.textMuted), "#8a8a8a")
        compare(String(host.theme.accent), "#3584e4")
        compare(String(host.theme.accentForeground), "#ffffff")
        verify(host.theme.surfaceRaised.hsvValue > host.theme.surface.hsvValue,
               "深色下抬升表面应比内容底色更亮")
        fuzzyCompare(host.theme.accentSoft.a, 0.22, 0.001)
        fuzzyCompare(host.theme.accentGhost.a, 0.14, 0.001)
    }

    function test_light_palette_restores_light_tokens() {
        useDarkPalette()
        host.palette.base = "#ffffff"
        host.palette.window = "#f2f2f2"
        host.palette.text = "#1f1f1f"
        host.palette.highlight = "#f5c211"
        host.palette.highlightedText = "#000000"
        wait(0)

        verify(!host.theme.darkMode)
        compare(String(host.theme.surface), "#ffffff")
        compare(String(host.theme.surfaceSheet), "#f2f2f2")
        compare(String(host.theme.textPrimary), "#1f1f1f")
        compare(String(host.theme.accent), "#f5c211")
        compare(String(host.theme.accentForeground), "#000000")
        fuzzyCompare(host.theme.accentSoft.a, 0.12, 0.001)
    }
}
