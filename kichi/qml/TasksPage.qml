import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 离线任务页：粘贴磁力/URL 回车提交，列表由后台轮询自动刷新。
// 顶层用 Item 而非 Page：Page 自带 header/footer，放进 StackLayout 容易撑爆高度。
Item {
    id: page

    property var backend: null
    property string pendingDeleteId: ""
    property string pendingDeleteName: ""

    // 页面尺寸常量：滚动条车道（见 KScrollBar / docs/pitfalls.md 第 13 条）
    UiMetrics {
        id: metrics
    }

    readonly property int activeCount: {
        var tasks = page.backend ? page.backend.tasks : []
        var n = 0
        for (var i = 0; i < tasks.length; i++) {
            if (!tasks[i].settled)
                n++
        }
        return n
    }

    ColumnLayout {
        anchors.fill: parent
        // 右边不留：列表铺到窗口右缘，滚动条贴边；
        // 输入行和状态行自己补右边距
        anchors.leftMargin: metrics.pageMargin
        anchors.topMargin: metrics.pageMargin
        anchors.bottomMargin: metrics.pageMargin
        spacing: 8

        RowLayout {
            Layout.fillWidth: true
            Layout.rightMargin: metrics.pageMargin
            spacing: 8

            TextField {
                id: urlField
                Layout.fillWidth: true
                placeholderText: qsTr("粘贴磁力链接或下载地址，回车提交")
                selectByMouse: true
                text: page.backend ? page.backend.taskInput : ""
                onAccepted: page.submit()
                onTextEdited: if (page.backend) page.backend.taskInput = text
            }

            Button {
                text: qsTr("提交")
                enabled: urlField.text.trim().length > 0
                onClicked: page.submit()
            }

            Button {
                text: qsTr("刷新")
                onClicked: page.backend.refreshTasks()
            }
        }

        Label {
            visible: page.activeCount > 0
            Layout.rightMargin: metrics.pageMargin
            text: qsTr("%1 条任务进行中，列表每 10 秒自动刷新").arg(page.activeCount)
            color: palette.placeholderText
            font.pixelSize: 12
        }

        ListView {
            id: list
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            spacing: 8
            model: page.backend ? page.backend.tasks : []

            // 列表铺到窗口右缘，条画在右侧 10px 车道里
            ScrollBar.vertical: KScrollBar {}

            delegate: Frame {
                id: card
                // 卡片宽度减去滚动条车道，右边不会压到条
                width: list.width - metrics.scrollBarSlot
                padding: 12

                readonly property var info: modelData
                readonly property bool isActive: info ? !info.settled : false

                ColumnLayout {
                    anchors.fill: parent
                    spacing: 6

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 10

                        Label {
                            Layout.fillWidth: true
                            text: card.info ? card.info.name : ""
                            elide: Text.ElideMiddle
                            font.bold: true
                        }

                        Label {
                            readonly property string phase: card.info ? card.info.phase : ""
                            text: card.info ? card.info.phaseText : ""
                            color: phase === "error" ? "#d9534f"
                                 : phase === "completed" ? "#3c9d5a"
                                 : palette.text
                            font.pixelSize: 12
                        }

                        Button {
                            text: qsTr("删除")
                            flat: true
                            onClicked: {
                                page.pendingDeleteId = card.info ? card.info.id : ""
                                page.pendingDeleteName = card.info ? card.info.name : ""
                                deleteDialog.open()
                            }
                        }
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 10
                        visible: card.isActive || (card.info && card.info.phase === "error")

                        KProgressBar {
                            Layout.fillWidth: true
                            from: 0
                            to: 100
                            value: card.info ? card.info.progress : 0
                            indeterminate: card.info && card.info.phase === "pending"
                        }

                        Label {
                            text: card.info ? card.info.progressText : ""
                            font.pixelSize: 12
                            color: palette.placeholderText
                        }
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 12

                        Label {
                            text: card.info ? card.info.sizeText : ""
                            font.pixelSize: 12
                            color: palette.placeholderText
                        }

                        Label {
                            text: card.info ? card.info.createdText : ""
                            font.pixelSize: 12
                            color: palette.placeholderText
                        }

                        Label {
                            Layout.fillWidth: true
                            text: card.info ? card.info.detail : ""
                            font.pixelSize: 12
                            elide: Text.ElideRight
                            color: card.info && card.info.phase === "error"
                                   ? "#d9534f" : palette.placeholderText
                        }
                    }
                }
            }

            Label {
                anchors.centerIn: parent
                visible: list.count === 0
                text: qsTr("暂无离线任务\n把磁力链接或下载地址粘到上方即可开始")
                horizontalAlignment: Text.AlignHCenter
                color: palette.placeholderText
            }
        }
    }

    function submit() {
        var url = urlField.text.trim()
        if (url.length === 0)
            return
        page.backend.addTask(url)
    }

    Dialog {
        id: deleteDialog
        title: qsTr("删除任务")
        modal: true
        anchors.centerIn: parent
        standardButtons: Dialog.Ok | Dialog.Cancel

        Label {
            width: 380
            wrapMode: Text.Wrap
            text: qsTr("确定删除任务「%1」吗？\n只会移除这条任务记录，已下载的云端文件不会删除。")
                .arg(page.pendingDeleteName)
        }

        onAccepted: page.backend.deleteTask(page.pendingDeleteId)
    }
}
