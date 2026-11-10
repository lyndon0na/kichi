import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "../components"
import "../core"

// 设置对话框：应用内模态浮层，不是独立窗口。
// 背景压暗主界面，打开期间不能操作其它页面；关闭后内容保留，下次打开还是上次的分组。
// 壳子是「卡片式设置」：大圆角面板、面板内自带标题栏和关闭按钮，左侧大纲不铺底色。
// ⚠ 弹窗必须挂 Overlay.overlay。
Popup {
    id: settingsDialog

    property var backend: null

    /// 退出登录：确认框由 main.qml 的 SignOutDialog 承担
    signal signOutRequested()

    // 同 KDialog：Popup 不是 Item，主题令牌挂属性、以 background 为 parent，
    // 避免被当成正文子项参与布局。
    readonly property KTheme theme: KTheme { parent: settingsDialog.background }

    parent: Overlay.overlay
    anchors.centerIn: Overlay.overlay
    width: settingsDialog.parent ? Math.min(940, settingsDialog.parent.width - 64) : 940
    height: settingsDialog.parent ? Math.min(680, settingsDialog.parent.height - 64) : 680

    padding: 0
    modal: true
    // 设置全部即时保存，点空白处或按 Esc 都可以直接关掉。
    closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside

    // 遮罩放在 Popup 背景里、铺满整个窗口：进出时跟面板同属一个弹层，
    // 由弹层的 opacity 过渡一起淡入淡出，不会出现「遮罩先没、面板还在」。
    // ⚠ KDE 的 org.kde.desktop 风格没有注册 Overlay.modal，不能靠 Popup.dim。
    background: Item {
        Rectangle {
            x: -settingsDialog.x
            y: -settingsDialog.y
            width: settingsDialog.parent ? settingsDialog.parent.width : 0
            height: settingsDialog.parent ? settingsDialog.parent.height : 0
            color: Qt.rgba(0, 0, 0, 0.4)
        }

        // 面板比普通弹窗再圆一档，和内部的 6px 控件拉开层次。
        Rectangle {
            anchors.fill: parent
            radius: theme.radiusDialog
            color: theme.surface
            border.width: 1
            border.color: theme.hairline
        }
    }

    contentItem: ColumnLayout {
        spacing: 0

        // 面板内标题栏：标题在左、关闭在右，关掉设置不必再摸 Esc。
        Item {
            Layout.fillWidth: true
            Layout.preferredHeight: 64

            Label {
                anchors.left: parent.left
                anchors.leftMargin: 28
                anchors.verticalCenter: parent.verticalCenter
                text: qsTr("设置")
                font.pixelSize: theme.fontTitle
                font.weight: Font.DemiBold
                color: theme.textPrimary
            }

            KIconButton {
                objectName: "settingsClose"
                anchors.right: parent.right
                anchors.rightMargin: 18
                anchors.verticalCenter: parent.verticalCenter
                iconName: "x"
                iconSize: 16
                iconOpacity: 0.7
                onClicked: settingsDialog.close()
            }
        }

        SettingsPage {
            objectName: "settingsPage"
            Layout.fillWidth: true
            Layout.fillHeight: true
            backend: settingsDialog.backend
            onSignOutRequested: settingsDialog.signOutRequested()
        }
    }
}
