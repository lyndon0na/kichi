import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 添加离线任务：每行一个磁力 / ed2k / HTTP 链接。
KDialog {
    id: dialog

    property var backend: null

    objectName: "newTaskDialog"
    title: qsTr("新任务")
    modal: true
    parent: Overlay.overlay
    anchors.centerIn: Overlay.overlay
    width: 520 + leftPadding + rightPadding
    spacing: theme.gapMd
    closePolicy: Popup.CloseOnEscape

    onOpened: {
        input.text = ""
        input.forceActiveFocus()
    }

    footer: DialogFooter {
        outerMargin: dialog.padding
        statusText: qsTr("支持磁力、ed2k 和 HTTP 下载地址；空行会自动忽略。")
        statusColor: theme.textMuted
        okText: qsTr("添加")
        okObjectName: "newTaskDialogOk"
        okEnabled: input.text.trim().length > 0
        onOkClicked: dialog.submit()
        onCancelClicked: dialog.close()
    }

    ColumnLayout {
        width: 520
        spacing: theme.gapMd

        Label {
            Layout.fillWidth: true
            text: qsTr("每行粘贴一个链接")
            color: theme.textPrimary
            font.pixelSize: theme.fontBody
            font.weight: Font.DemiBold
        }

        Item {
            Layout.fillWidth: true
            Layout.preferredHeight: 180

            Rectangle {
                anchors.fill: parent
                radius: theme.radiusControl
                color: theme.surfaceRaised
                border.width: input.activeFocus ? 2 : 1
                border.color: input.activeFocus ? theme.accent
                            : input.hovered ? Qt.rgba(palette.text.r, palette.text.g,
                                                       palette.text.b, 0.22)
                            : theme.hairline
            }

            ScrollView {
                id: scroll

                anchors.fill: parent
                anchors.margins: 1
                clip: true
                background: Rectangle { color: "transparent" }
                ScrollBar.vertical: KScrollBar {}
                ScrollBar.horizontal.policy: ScrollBar.AlwaysOff

                TextArea {
                    id: input

                    objectName: "newTaskInput"
                    width: scroll.availableWidth
                    padding: 11
                    color: theme.textPrimary
                    placeholderText: qsTr("magnet:?xt=...\nhttps://...")
                    placeholderTextColor: theme.textMuted
                    font.pixelSize: theme.fontBody
                    selectByMouse: true
                    wrapMode: TextArea.Wrap
                    background: Rectangle { color: "transparent" }
                }
            }
        }

    }

    function submit() {
        var text = input.text.trim()
        if (text.length === 0 || !dialog.backend)
            return
        dialog.backend.addTask(text)
        dialog.close()
    }
}
