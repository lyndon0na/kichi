import QtQuick
import QtQuick.Controls

// 统一弹窗外观：大圆角、弹窗表面色、极浅描边。
//
// 只覆盖 background 和默认内边距，modal / accepted / standardButtons /
// footer 等行为全部沿用 QQC2 Dialog，不改变任何交互（见 pitfalls 第 28 条）。
// 各弹窗仍自己负责 parent: Overlay.overlay + 居中（pitfalls 第 10 条）。
Dialog {
    id: control

    // ⚠ 主题令牌必须挂在**属性**上，不能写成普通子项 `KTheme { id: theme }`：
    // Dialog 的默认 contentItem 只在「恰好一个子项」时才认内容，多一个不可见
    // 子项正文就整个塌成 0——弹窗外壳照常居中，输入框被挤成 0 高，页脚按钮
    // 直接压在它上面（2026-10-29 用户截图：所有弹窗都这样）。
    // 同源问题见 docs/pitfalls.md 第 36 条（Frame）。子类直接写 `theme.xxx` 即可。
    readonly property KTheme theme: KTheme { }

    padding: 18

    background: Rectangle {
        radius: theme.radiusSurface
        color: theme.surfaceSheet
        border.width: 1
        border.color: theme.hairline
    }
}
