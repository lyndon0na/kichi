import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 分享创建成功后的结果区：链接和密码放在同一个内容块里。
// 操作按钮和复制提示统一放在弹窗页脚，剪贴板文本由外层生成。
ColumnLayout {
    id: pane

    property string itemLabel: ""
    property string expirationText: ""
    property string shareText: ""
    property bool hasPassword: false

    spacing: 10

    KTheme { id: theme }

    Label {
        Layout.fillWidth: true
        text: qsTr("%1的分享链接已创建，有效期：%2。")
            .arg(pane.itemLabel)
            .arg(pane.expirationText)
        wrapMode: Text.Wrap
        font.pixelSize: theme.fontBody
        color: theme.textPrimary
    }

    Label {
        Layout.fillWidth: true
        text: pane.hasPassword ? qsTr("分享链接和访问密码") : qsTr("分享链接")
        font.pixelSize: theme.fontMeta
        color: theme.textMuted
    }

    Rectangle {
        Layout.fillWidth: true
        Layout.preferredHeight: 76
        radius: theme.radiusControl
        color: theme.surfaceRaised
        border.width: 1
        border.color: theme.hairline

        TextEdit {
            objectName: "shareCreatedContent"
            anchors.fill: parent
            anchors.margins: theme.gapMd
            text: pane.shareText
            readOnly: true
            selectByMouse: true
            textFormat: TextEdit.PlainText
            wrapMode: TextEdit.WrapAnywhere
            color: theme.textPrimary
            font.pixelSize: theme.fontBody
            selectionColor: theme.accent
            selectedTextColor: theme.onAccent
        }
    }
}
