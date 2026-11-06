import QtQuick
import QtQuick.Controls

// 退出登录确认框：由设置对话框「账号」触发，确认后由 main.qml 清令牌。
KDialog {
    id: dialog

    signal confirmed()

    parent: Overlay.overlay
    anchors.centerIn: Overlay.overlay
    title: qsTr("退出登录")
    modal: true
    width: 360 + leftPadding + rightPadding

    footer: DialogFooter {
        outerMargin: dialog.padding
        onOkClicked: {
            dialog.close()
            dialog.confirmed()
        }
        onCancelClicked: dialog.close()
    }

    Label {
        text: qsTr("将清除本机保存的登录令牌，下次需要重新输入账号密码。确定退出？")
        wrapMode: Text.Wrap
        width: 360
    }
}
