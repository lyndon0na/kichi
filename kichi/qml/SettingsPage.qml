import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 设置页：播放 / 下载 / 通知 / 窗口。
// 顶层用 Item 而非 Page：Page 自带 header/footer，放进 StackLayout 容易撑爆高度。
// 所有改动即时生效并落盘（backend 侧负责持久化与错误提示）。
Item {
    id: page

    property var backend: null

    // 下载目录输入框在编辑中时不要被 backend 的刷新覆盖（否则打字会被打断）
    property bool downloadDirty: false

    // 页面尺寸常量：滚动条车道（见 KScrollBar / docs/pitfalls.md 第 13 条）
    UiMetrics {
        id: metrics
    }

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
            spacing: 16

            // ── 播放（内容在 SettingsPlayback.qml） ────────────
            SettingsPlayback {
                Layout.fillWidth: true
                backend: page.backend
            }

            // ── 下载 ───────────────────────────────────────────
            Label {
                text: qsTr("下载")
                font.bold: true
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: 12

                Label {
                    text: qsTr("下载目录")
                    Layout.preferredWidth: 110
                }

                TextField {
                    id: downloadField
                    Layout.fillWidth: true
                    selectByMouse: true
                    placeholderText: qsTr("留空 = ~/Downloads")
                    text: page.backend ? page.backend.downloadDir : ""
                    onTextEdited: page.downloadDirty = true
                    onEditingFinished: page.applyDownloadDir()
                }

                RowLayout {
                    spacing: 6

                    Button {
                        text: qsTr("应用")
                        enabled: page.downloadDirty
                        onClicked: page.applyDownloadDir()
                    }
                    Button {
                        text: qsTr("打开目录")
                        flat: true
                        onClicked: {
                            if (page.backend)
                                page.backend.openDownloadDir()
                        }
                    }
                }
            }

            // ── 通知 ───────────────────────────────────────────
            Label {
                text: qsTr("通知")
                font.bold: true
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: 12

                Label {
                    text: qsTr("任务完成通知")
                    Layout.preferredWidth: 110
                }
                CheckBox {
                    text: qsTr("离线任务完成或失败时弹出系统通知")
                    checked: page.backend ? page.backend.notifyEnabled : true
                    onToggled: function () {
                        if (page.backend)
                            page.backend.setNotifyEnabled(checked)
                    }
                }
                Item { Layout.fillWidth: true }
            }

            // ── 窗口 ───────────────────────────────────────────
            Label {
                text: qsTr("窗口")
                font.bold: true
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: 12

                Label {
                    text: qsTr("关闭窗口")
                    Layout.preferredWidth: 110
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

            // ── 存储信息（只读；内容在 SettingsAbout.qml） ──────
            SettingsAbout {
                Layout.fillWidth: true
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
