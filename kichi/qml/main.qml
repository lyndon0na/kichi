import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Qt.labs.platform as Platform

// 由 build.rs 的 QmlModule("io.kichi") 生成
import io.kichi 1.0

ApplicationWindow {
    id: root

    title: "kichi"
    // 上次关窗时的尺寸（0 = 没存过，用默认）
    width: backend.windowWidth > 0 ? backend.windowWidth : 1000
    height: backend.windowHeight > 0 ? backend.windowHeight : 640
    visible: true
    color: palette.window

    // 会话对象：构造时自动恢复 KWallet 里的登录态
    Kichi {
        id: backend
    }

    property bool signedIn: backend.authState === "signedIn"
    /// 当前是否最大化（Wayland 下窗口状态由混成器管，这里跟踪自己的意图）
    property bool isMaximized: false
    /// 缩到托盘后窗口隐藏，这里记住「是托盘收起来的」，不是用户主动关
    property bool hiddenToTray: false

    Component.onCompleted: {
        backend.warmUp()
        if (backend.windowMaximized)
            showMaximized()
        else
            isMaximized = false
    }

    onWidthChanged: saveGeometryTimer.restart()
    onHeightChanged: saveGeometryTimer.restart()
    onVisibilityChanged: {
        if (visibility === Window.Maximized || visibility === Window.FullScreen)
            isMaximized = true
        else if (visibility === Window.Windowed)
            isMaximized = false
        saveGeometryTimer.restart()
    }

    // 拖窗口边缘会连续触发尺寸变化：防抖后再落盘
    Timer {
        id: saveGeometryTimer
        interval: 500
        onTriggered: {
            var w = root.isMaximized ? backend.windowWidth : root.width
            var h = root.isMaximized ? backend.windowHeight : root.height
            backend.saveWindowGeometry(w, h, root.isMaximized)
        }
    }

    onClosing: function (close) {
        if (backend.closeToTray && trayIcon.available) {
            close.accepted = false
            saveGeometryTimer.stop()
            // 最大化时保留上一次普通尺寸，别把最大化尺寸当成还原尺寸
            var w = root.isMaximized ? backend.windowWidth : root.width
            var h = root.isMaximized ? backend.windowHeight : root.height
            backend.saveWindowGeometry(w, h, root.isMaximized)
            root.hiddenToTray = true
            root.hide()
        }
    }

    // —— 托盘（仅「关闭到托盘」打开时出现；Weston 等无托盘宿主的环境自动不显示）——
    Platform.SystemTrayIcon {
        id: trayIcon

        visible: backend.closeToTray && available
        tooltip: "kichi"

        icon.name: "folder-cloud"

        menu: Platform.Menu {
            Platform.MenuItem {
                text: qsTr("显示 kichi")
                onTriggered: root.restoreFromTray()
            }
            Platform.MenuItem {
                text: qsTr("退出")
                onTriggered: Qt.quit()
            }
        }

        onActivated: function (reason) {
            if (reason === Platform.SystemTrayIcon.Trigger
                    || reason === Platform.SystemTrayIcon.DoubleClick)
                root.restoreFromTray()
        }
    }

    function restoreFromTray() {
        if (!root.hiddenToTray)
            return
        root.hiddenToTray = false
        root.show()
        root.raise()
        root.requestActivate()
    }

    // 后台线程的结果由主线程抽水投递（cxx-qt 0.10 下不能用 qsignal）
    Timer {
        interval: 100
        running: true
        repeat: true
        onTriggered: backend.pump()
    }

    header: ToolBar {
        RowLayout {
            anchors.fill: parent
            spacing: 12

            Label {
                text: "kichi"
                font.bold: true
            }

            Item { Layout.fillWidth: true }

            Label {
                text: backend.account
                visible: root.signedIn && backend.account.length > 0
                color: palette.placeholderText
            }

            BusyIndicator {
                running: backend.busy
                visible: running
                Layout.preferredWidth: 24
                Layout.preferredHeight: 24
            }

            Label {
                text: backend.quotaSummary
                visible: backend.quotaSummary.length > 0
                elide: Text.ElideRight
                Layout.maximumWidth: 260
            }

            Button {
                text: qsTr("退出登录")
                visible: root.signedIn
                onClicked: signOutDialog.open()
            }
        }
    }

    StackLayout {
        anchors.fill: parent
        anchors.margins: 24
        currentIndex: root.signedIn ? 1 : 0

        LoginPage {
            backend: backend
        }

        ColumnLayout {
            spacing: 0

            TabBar {
                id: mainTabs
                Layout.fillWidth: true
                visible: root.signedIn

                TabButton {
                    text: qsTr("文件")
                }
                TabButton {
                    text: qsTr("离线任务")
                }
                TabButton {
                    text: qsTr("设置")
                }
            }

            StackLayout {
                Layout.fillWidth: true
                Layout.fillHeight: true
                currentIndex: mainTabs.currentIndex

                FilesPage {
                    backend: backend
                }

                TasksPage {
                    backend: backend
                }

                SettingsPage {
                    backend: backend
                }
            }
        }
    }

    // 状态消息（错误 / 进度 / 提示）
    footer: Frame {
        visible: backend.statusMessage.length > 0
        padding: 10

        Label {
            text: backend.statusMessage
            wrapMode: Text.Wrap
            width: parent.width
        }
    }

    Dialog {
        id: signOutDialog
        title: qsTr("退出登录")
        modal: true
        anchors.centerIn: parent
        standardButtons: Dialog.Ok | Dialog.Cancel

        Label {
            text: qsTr("将清除本机保存的登录令牌，下次需要重新输入账号密码。确定退出？")
            wrapMode: Text.Wrap
            width: 360
        }

        onAccepted: backend.signOut()
    }
}
