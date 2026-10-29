import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 设置页：播放 / 下载 / 通知 / 窗口。
// 顶层用 Item 而非 Page：Page 自带 header/footer，放进 StackLayout 容易撑爆高度。
// 所有改动即时生效并落盘（backend 侧负责持久化与错误提示）。
Item {
    id: page

    property var backend: null

    /// 退出登录：设置页只负责发信号，确认框与清令牌在 main.qml / backend 那边
    signal signOutRequested()

    // 下载目录输入框在编辑中时不要被 backend 的刷新覆盖（否则打字会被打断）
    property bool downloadDirty: false

    // 页面尺寸常量：滚动条车道（见 KScrollBar / docs/pitfalls.md 第 13 条）
    UiMetrics {
        id: metrics
    }

    KTheme { id: theme }

    Flickable {
        id: scroller
        anchors.fill: parent
        // 右边不留：视图铺到窗口右缘，滚动条贴边；内容列自己让出车道
        anchors.leftMargin: metrics.pageMargin
        anchors.topMargin: metrics.pageMargin
        anchors.bottomMargin: metrics.pageMargin
        clip: true
        contentWidth: width
        contentHeight: body.implicitHeight
        boundsBehavior: Flickable.StopAtBounds

        // 设置页以前完全没挂滚动条（滚轮能滚，但看不出能滚、也拖不动）
        ScrollBar.vertical: KScrollBar {}

        ColumnLayout {
            id: body
            // 内容宽度减去滚动条车道：文字停在条左边
            width: scroller.width - metrics.scrollBarSlot
            spacing: theme.gapMd

            // ── 播放（内容在 SettingsPlayback.qml） ────────────
            SettingsPlayback {
                Layout.fillWidth: true
                backend: page.backend
            }

            // ── 下载 ───────────────────────────────────────────
            KSectionTitle {
                text: qsTr("下载")
                Layout.fillWidth: true
                Layout.topMargin: theme.gapMd
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: theme.gapMd

                Label {
                    text: qsTr("下载目录")
                    Layout.preferredWidth: 96
                    font.pixelSize: theme.fontBody
                    color: theme.textPrimary
                }

                KTextField {
                    id: downloadField
                    Layout.fillWidth: true
                    selectByMouse: true
                    placeholderText: qsTr("留空 = ~/Downloads")
                    text: page.backend ? page.backend.downloadDir : ""
                    onTextEdited: page.downloadDirty = true
                    onEditingFinished: page.applyDownloadDir()
                }

                RowLayout {
                    spacing: theme.gapSm

                    KButton {
                        text: qsTr("应用")
                        variant: "primary"
                        implicitHeight: 32
                        enabled: page.downloadDirty
                        onClicked: page.applyDownloadDir()
                    }
                    KButton {
                        text: qsTr("打开目录")
                        variant: "ghost"
                        implicitHeight: 32
                        onClicked: {
                            if (page.backend)
                                page.backend.openDownloadDir()
                        }
                    }
                }
            }

            // ── 通知 ───────────────────────────────────────────
            KSectionTitle {
                text: qsTr("通知")
                Layout.fillWidth: true
                Layout.topMargin: theme.gapMd
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: theme.gapMd

                Label {
                    text: qsTr("任务结束通知")
                    Layout.preferredWidth: 96
                    font.pixelSize: theme.fontBody
                    color: theme.textPrimary
                }
                CheckBox {
                    text: qsTr("下载或离线任务结束时弹出系统通知")
                    checked: page.backend ? page.backend.notifyEnabled : true
                    onToggled: function () {
                        if (page.backend)
                            page.backend.setNotifyEnabled(checked)
                    }
                }
                Item { Layout.fillWidth: true }
            }

            // ── 窗口 ───────────────────────────────────────────
            KSectionTitle {
                text: qsTr("窗口")
                Layout.fillWidth: true
                Layout.topMargin: theme.gapMd
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: theme.gapMd

                Label {
                    text: qsTr("关闭窗口")
                    Layout.preferredWidth: 96
                    font.pixelSize: theme.fontBody
                    color: theme.textPrimary
                }
                CheckBox {
                    text: qsTr("最小化到托盘（任务轮询与通知继续）")
                    checked: page.backend ? page.backend.closeToTray : false
                    onToggled: function () {
                        if (page.backend)
                            page.backend.setCloseToTray(checked)
                    }
                }
                Item { Layout.fillWidth: true }
            }

            // ── 账号 ───────────────────────────────────────────
            // 退出登录放这里而不是侧栏账号卡：低频、不可逆，离日常操作远一点
            // （2026-10-29 用户要求）。
            KSectionTitle {
                text: qsTr("账号")
                Layout.fillWidth: true
                Layout.topMargin: theme.gapMd
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: theme.gapMd

                Label {
                    Layout.fillWidth: true
                    wrapMode: Text.Wrap
                    text: qsTr("退出后本机保存的登录令牌会被清除，下次需要重新输入账号密码。")
                    font.pixelSize: theme.fontMeta
                    color: theme.textMuted
                }

                KButton {
                    objectName: "settingsSignOut"
                    text: qsTr("退出登录")
                    variant: "danger"
                    implicitHeight: 32
                    onClicked: page.signOutRequested()
                }
            }

            // ── 存储信息（只读；内容在 SettingsAbout.qml） ──────
            SettingsAbout {
                Layout.fillWidth: true
                Layout.topMargin: theme.gapMd
                backend: page.backend
            }

            Item { Layout.fillHeight: true }
        }
    }

    function applyDownloadDir() {
        if (!page.backend)
            return
        page.downloadDirty = false
        page.backend.setDownloadDir(downloadField.text)
        // 后端可能因为非法路径拒绝，读回真实生效值
        downloadField.text = page.backend.downloadDir
    }
}
