import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 设置页：播放 / 通知 / 窗口 / 文件列表。
// 顶层用 Item 而非 Page：Page 自带 header/footer，放进 StackLayout 容易撑爆高度。
// 所有改动即时生效并落盘（backend 侧负责持久化与错误提示）。
Item {
    id: page

    property var backend: null

    // 输入框在编辑中时不要被 backend 的刷新覆盖（否则打字会被打断）
    property bool commandDirty: false
    property bool argsDirty: false

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

            // ── 播放 ───────────────────────────────────────────
            Label {
                text: qsTr("播放")
                font.bold: true
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: 12

                Label {
                    text: qsTr("默认清晰度")
                    Layout.preferredWidth: 110
                }

                ComboBox {
                    id: qualityBox
                    Layout.preferredWidth: 220
                    model: [
                        { "value": "ask", "label": qsTr("每次询问（弹出档位菜单）") },
                        { "value": "original", "label": qsTr("优先原画") },
                        { "value": "transcode", "label": qsTr("优先最高转码档") }
                    ]
                    textRole: "label"
                    valueRole: "value"

                    Component.onCompleted: syncFromBackend()
                    Connections {
                        target: page.backend
                        function onChanged() {
                            qualityBox.syncFromBackend()
                        }
                    }
                    function syncFromBackend() {
                        if (!page.backend)
                            return
                        var index = qualityBox.indexOfValue(page.backend.preferredQuality)
                        if (index >= 0 && index !== qualityBox.currentIndex)
                            qualityBox.currentIndex = index
                    }
                    onActivated: if (page.backend) page.backend.setPreferredQuality(currentValue)
                }

                Label {
                    Layout.fillWidth: true
                    wrapMode: Text.Wrap
                    color: palette.placeholderText
                    font.pixelSize: 12
                    text: qsTr("选「优先」后点播放直接起播；所偏好的档位不可用时会回退到档位菜单。")
                }
            }

            GridLayout {
                Layout.fillWidth: true
                columns: 3
                columnSpacing: 12
                rowSpacing: 8

                Label {
                    text: qsTr("播放器命令")
                    Layout.preferredWidth: 110
                }
                TextField {
                    id: commandField
                    Layout.fillWidth: true
                    selectByMouse: true
                    placeholderText: page.backend ? page.backend.playerDefaultCommand : ""
                    text: page.backend ? page.backend.playerCommand : ""
                    onTextEdited: page.commandDirty = true
                    onEditingFinished: page.applyPlayer()
                }
                Label { text: "" }

                Label {
                    text: qsTr("附加参数")
                    Layout.preferredWidth: 110
                }
                TextField {
                    id: argsField
                    Layout.fillWidth: true
                    selectByMouse: true
                    placeholderText: page.backend ? page.backend.playerDefaultArgs : ""
                    text: page.backend ? page.backend.playerArgs : ""
                    onTextEdited: page.argsDirty = true
                    onEditingFinished: page.applyPlayer()
                }
                RowLayout {
                    spacing: 6

                    Button {
                        text: qsTr("应用")
                        enabled: page.commandDirty || page.argsDirty
                        onClicked: page.applyPlayer()
                    }
                    Button {
                        text: qsTr("恢复默认")
                        flat: true
                        onClicked: {
                            page.commandDirty = false
                            page.argsDirty = false
                            if (page.backend)
                                page.backend.resetPlayer()
                        }
                    }
                }
            }

            Label {
                Layout.fillWidth: true
                wrapMode: Text.Wrap
                color: palette.placeholderText
                font.pixelSize: 12
                text: qsTr("默认：%1 %2。留空表示使用默认；文件名用 {title} 占位，直链走临时清单不进命令行。")
                        .arg(page.backend ? page.backend.playerDefaultCommand : "")
                        .arg(page.backend ? page.backend.playerDefaultArgs : "")
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
                    onToggled: if (page.backend) page.backend.setNotifyEnabled(checked)
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
                    onToggled: if (page.backend) page.backend.setCloseToTray(checked)
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
                    onActivated: if (page.backend)
                        page.backend.setSorting(currentValue, ascendingBox.checked)
                }

                CheckBox {
                    id: ascendingBox
                    text: qsTr("升序")
                    checked: page.backend ? page.backend.sortAscending : true
                    onToggled: if (page.backend)
                        page.backend.setSorting(sortBox.currentValue, checked)
                }

                Label {
                    Layout.fillWidth: true
                    wrapMode: Text.Wrap
                    color: palette.placeholderText
                    font.pixelSize: 12
                    text: qsTr("文件夹始终排在前面。")
                }
            }

            // ── 存储信息（只读） ───────────────────────────────
            Label {
                text: qsTr("关于")
                font.bold: true
            }

            GridLayout {
                Layout.fillWidth: true
                columns: 2
                columnSpacing: 12
                rowSpacing: 6

                Label {
                    text: qsTr("令牌存储")
                    color: palette.placeholderText
                }
                Label {
                    text: page.backend ? page.backend.storageBackend : ""
                    Layout.fillWidth: true
                    wrapMode: Text.Wrap
                }

                Label {
                    text: qsTr("设置文件")
                    color: palette.placeholderText
                }
                Label {
                    text: page.backend ? page.backend.settingsPath : ""
                    Layout.fillWidth: true
                    wrapMode: Text.WrapAnywhere
                    font.family: "monospace"
                    font.pixelSize: 12
                }
            }

            Item { Layout.fillHeight: true }
        }
    }

    function applyPlayer() {
        if (!page.backend)
            return
        page.commandDirty = false
        page.argsDirty = false
        page.backend.setPlayer(commandField.text, argsField.text)
    }
}
