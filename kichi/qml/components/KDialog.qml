import QtQuick
import QtQuick.Controls
import "../core"

// 统一弹窗外观：大圆角、弹窗表面色、极浅描边。
//
// 只覆盖 background 和默认内边距，modal / accepted / standardButtons /
// footer 等行为全部沿用 QQC2 Dialog，不改变任何交互。
// 各弹窗仍自己负责 parent: Overlay.overlay + 居中。
Dialog {
    id: control

    // ⚠ 主题令牌必须挂在**属性**上，不能写成普通子项 `KTheme { id: theme }`：
    // Dialog 的默认 contentItem 只在「恰好一个子项」时才认内容，多一个不可见
    // 子项正文就整个塌成 0——弹窗外壳照常居中，输入框被挤成 0 高，页脚按钮
    // 直接压在它上面（2026-10-29 用户截图：所有弹窗都这样）。
    // 同源问题出在 Frame 上。Popup 自身不是 Item；
    // 挂 background 可以实时继承弹窗 palette，又不参与正文 / 页脚布局或 overlay 事件。
    readonly property KTheme theme: KTheme { parent: control.background }

    padding: 18
    // 标题 / 正文 / 页脚统一使用同一档间隔；各弹窗有特殊需要时再覆盖。
    spacing: theme.gapMd
    // 标题栏自己已有上方留白；正文不要再从 topPadding 叠一层。
    // 无标题的防御分支仍保留正常上边距。
    topPadding: title.length > 0 ? 0 : padding

    // 标题栏自己画：KDE 的 DialogHeader 走 Kirigami / kdeglobals，非 KDE 桌面
    // 既不会跟随注入的调色板，关闭按钮用的 window-close-symbolic 也未必存在。
    // 这里只保留标题和关闭入口，颜色全部走 KTheme（= 当前 palette）。
    header: Item {
        id: header

        visible: control.title.length > 0
        implicitHeight: visible
                        ? Math.max(titleLabel.implicitHeight,
                                   closeButton.implicitHeight) + theme.gapMd
                        : 0

        ElidedLabel {
            id: titleLabel

            objectName: "dialogTitleLabel"
            anchors.left: parent.left
            anchors.leftMargin: control.padding
            anchors.right: closeButton.left
            anchors.rightMargin: theme.gapSm
            anchors.verticalCenter: parent.verticalCenter
            text: control.title
            font.pixelSize: theme.fontSection
            font.weight: Font.DemiBold
            color: theme.textPrimary
            elide: Text.ElideRight
        }

        KIconButton {
            id: closeButton

            objectName: "dialogCloseButton"
            anchors.right: parent.right
            anchors.rightMargin: control.padding
            anchors.verticalCenter: parent.verticalCenter
            iconName: "x"
            iconSize: 14
            iconOpacity: 0.7
            onClicked: control.reject()
        }
    }

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
