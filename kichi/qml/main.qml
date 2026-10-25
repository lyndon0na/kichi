import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 由 build.rs 的 QmlModule("io.kichi") 生成
import io.kichi 1.0

ApplicationWindow {
    id: root

    title: "kichi"
    width: 1000
    height: 640
    visible: true
    color: palette.window

    // 会话对象：构造时自动恢复 KWallet 里的登录态
    Kichi {
        id: backend
    }

    property bool signedIn: backend.authState === "signedIn"

    Component.onCompleted: backend.warmUp()

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

            BusyIndicator {
                running: backend.busy
                visible: running
                Layout.preferredWidth: 24
                Layout.preferredHeight: 24
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

        HomePage {
            backend: backend
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
