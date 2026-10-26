import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 设置页：播放 / 通知 / 窗口 / 文件列表。
// 顶层用 Item 而非 Page：Page 自带 header/footer，放进 StackLayout 容易撑爆高度。
// 所有改动即时生效并落盘（backend 侧负责持久化与错误提示）。
Item {
    id: page

    property var backend: null

    // 下载目录输入框在编辑中时不要被 backend 的刷新覆盖（否则打字会被打断）
    property bool downloadDirty: false

    Flickable {
        id: scroller
        anchors.fill: parent
        anchors.margins: 12
        clip: true
        contentWidth: width
        contentHeight: body.implicitHeight
        boundsBehavior: Flickable.StopAtBounds

        ColumnLayout {
            id: body
            width: scroller.width
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

            Label {
                Layout.fillWidth: true
                wrapMode: Text.Wrap
                color: palette.placeholderText
                font.pixelSize: 12
                text: qsTr("支持 ~/ 开头。文件页点「⬇」直接下到这里；同名文件自动加序号，不会覆盖。")
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

            // ── 文件列表 ───────────────────────────────────────
            Label {
                text: qsTr("文件列表")
                font.bold: true
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: 12

                Label {
                    text: qsTr("排序方式")
                    Layout.preferredWidth: 110
                }
                ComboBox {
                    id: sortBox
                    Layout.preferredWidth: 180
                    model: [
                        { "value": "name", "label": qsTr("名称") },
                        { "value": "size", "label": qsTr("大小") },
                        { "value": "modified", "label": qsTr("修改时间") }
                    ]
                    textRole: "label"
                    valueRole: "value"

                    Component.onCompleted: syncFromBackend()
                    Connections {
                        target: page.backend
                        function onChanged() {
                            sortBox.syncFromBackend()
                        }
                    }
                    function syncFromBackend() {
                        if (!page.backend)
                            return
                        var index = sortBox.indexOfValue(page.backend.sortField)
                        if (index >= 0 && index !== sortBox.currentIndex)
                            sortBox.currentIndex = index
                    }
                    onActivated: function (index) {
                        if (!page.backend)
                            return
                        page.backend.setSorting(sortBox.model[index].value, ascendingBox.checked)
                    }
                }

                CheckBox {
                    id: ascendingBox
                    text: qsTr("升序")
                    checked: page.backend ? page.backend.sortAscending : true
                    onToggled: function () {
                        if (!page.backend)
                            return
                        page.backend.setSorting(sortBox.model[sortBox.currentIndex].value, checked)
                    }
                }

                Label {
                    Layout.fillWidth: true
                    wrapMode: Text.Wrap
                    color: palette.placeholderText
                    font.pixelSize: 12
                    text: qsTr("文件夹始终排在前面。")
                }
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
