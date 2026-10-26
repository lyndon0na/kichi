import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

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
    /// 真的要退出（托盘菜单「退出」/注销托盘后），不缩回托盘
    property bool reallyQuitting: false

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
        // 托盘点「退出」= 真退出，不能又被下面的缩托盘逻辑拦下来
        // （Qt.quit() 同样会触发 onClosing，不特判就会出现
        //  「点了退出图标还在、窗口也点不回来」——2026-10-26 用户实测）
        if (root.reallyQuitting) {
            backend.shutdownTray()
            return
        }
        if (backend.closeToTray && backend.trayAvailable) {
            close.accepted = false
            saveGeometryTimer.stop()
            // 最大化时保留上一次普通尺寸，别把最大化尺寸当成还原尺寸
            var w = root.isMaximized ? backend.windowWidth : root.width
            var h = root.isMaximized ? backend.windowHeight : root.height
            backend.saveWindowGeometry(w, h, root.isMaximized)
            root.hiddenToTray = true
            root.hide()
        } else {
            // 真退出：注销托盘，否则 KDE 面板留下点不动的僵尸图标
            backend.shutdownTray()
        }
    }

    // —— 托盘 ——
    // ⚠ 2026-10-26 实测：Qt.labs.platform 的 SystemTrayIcon 在 KDE 平台主题下
    // 会走 KStatusNotifierItem，它要构造 QMenu（QtWidgets），而本应用只有
    // QGuiApplication → qFatal → 启动即 abort。容器一变就崩，连「关掉设置」
    // 的机会都没有。已改用 Rust 侧 ksni（纯 D-Bus），见 cxxqt_object/tray.rs。

    // 托盘事件：用 Connections 而不是 onXxxChanged（后者对 cxx-qt 生成的
    // 信号名推导不可靠，实测会报 "Cannot assign to non-existent property"）
    Connections {
        target: backend

        function onChanged() {
            // 显示：托盘点了图标或「显示 kichi」
            if (backend.trayShowRequested > 0 && root.lastTrayShow !== backend.trayShowRequested) {
                root.lastTrayShow = backend.trayShowRequested
                root.restoreFromTray()
            }
            // 退出：托盘「退出」，真退出而不是缩回托盘。
            // 必须先注销托盘，否则 KDE 面板会留下点不动的僵尸图标
            if (backend.quitRequested) {
                root.reallyQuitting = true
                Qt.quit()
            }
        }
    }

    property int lastTrayShow: 0

    /// 从托盘唤回窗口。窗口本来就在前台时也执行 show/raise，当作「置顶」处理。
    function restoreFromTray() {
        root.hiddenToTray = false
        if (!root.visible)
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

    StackLayout {
        anchors.fill: parent
        currentIndex: root.signedIn ? 1 : 0

        LoginPage {
            backend: backend
        }

        RowLayout {
            spacing: 0

            SideBar {
                Layout.preferredWidth: root.width < 760 ? 168 : 208
                Layout.fillHeight: true
                backend: backend
                currentIndex: root.currentPage
                onPageRequested: function (index) {
                    root.currentPage = index
                }
                onSignOutRequested: signOutDialog.open()
            }

            ColumnLayout {
                Layout.fillWidth: true
                Layout.fillHeight: true
                spacing: 0

                // 页标题条：左边页名、中间搜索框（仅文件页）、右边目录操作。
                // 不用 Frame（Fusion 样式自带一圈边框），这里只要留白 + 文字。
                // 两侧各放一个撑开的空白，搜索框落在标题和按钮之间居中。
                RowLayout {
                    Layout.fillWidth: true
                    Layout.leftMargin: 14
                    Layout.rightMargin: 12
                    Layout.topMargin: 11
                    Layout.bottomMargin: 11
                    spacing: 8

                    Label {
                        id: titleLabel
                        text: root.pageTitle()
                        font.pixelSize: 20
                        font.bold: true
                    }

                    Item {
                        Layout.fillWidth: true
                    }

                    FilesPageSearchBar {
                        id: searchBar
                        visible: root.currentPage === 0
                        Layout.preferredWidth: 330
                        Layout.minimumWidth: 160
                        Layout.maximumWidth: 330
                        Layout.alignment: Qt.AlignVCenter
                        backend: backend
                    }

                    Item {
                        Layout.fillWidth: true
                    }

                    // 目录操作：只对文件页有意义。搜索视图里置灰（它们作用的是
                    // 当前目录，不是搜索结果），但保留占位，免得搜索框左右跳。
                    RowLayout {
                        visible: root.currentPage === 0
                        spacing: 5

                        ToolButton {
                            text: qsTr("刷新")
                            enabled: !filesPage.searching && !backend.busy
                            onClicked: backend.refreshFiles()
                        }

                        ToolButton {
                            text: qsTr("＋ 新建文件夹")
                            enabled: !filesPage.searching && !backend.busy
                                     && !backend.fileActionRunning
                            onClicked: filesPage.beginNewFolder()
                        }
                    }
                }

                StackLayout {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    currentIndex: root.currentPage

                    FilesPage {
                        id: filesPage
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

    property int currentPage: 0

    function pageTitle() {
        if (root.currentPage === 1)
            return qsTr("离线任务")
        if (root.currentPage === 2)
            return qsTr("设置")
        return qsTr("我的文件")
    }
}
