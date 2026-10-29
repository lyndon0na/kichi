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
    // 同源问题见 docs/pitfalls.md 第 36 条（Frame）。Popup 自身不是 Item；
    // 挂 background 可以实时继承弹窗 palette，又不参与正文 / 页脚布局或 overlay 事件。
    readonly property KTheme theme: KTheme { parent: control.background }

    padding: 18

    // org.kde.desktop 的 Dialog 样式会把 contentItem 固定成一个空 Item。
    // 空 Item 不会把子项高度汇总给弹窗，长正文就会溢出到页脚下面。
    // 宽度由各弹窗显式给出；这里只汇总高度，避免和 Dialog.implicitWidth 互相绑定。
    contentItem: Item {
        implicitHeight: childrenRect.height
    }

    background: Rectangle {
        radius: theme.radiusSurface
        color: theme.surfaceSheet
        border.width: 1
        border.color: theme.hairline
    }
}
