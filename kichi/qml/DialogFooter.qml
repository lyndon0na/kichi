pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 弹窗页脚：左「取消」、右「确定」（重命名 / 新建文件夹共用）。
//
// 为什么不用 `standardButtons`：QQC2 的 Dialog 点在 Ok（AcceptRole）上会自己
// `close()`，而这两个操作都是异步请求——失败原因回来时对话框已经不可见，
// 用户只看到「点了确定就消失了」。页脚按钮只发信号，由弹窗自己决定什么时候关：
// 成功收起，失败停在原地显示原因（见 `FilesPageMenu.qml` 的 onFileActionDone）。
RowLayout {
    id: footer

    /// 「确定」能不能点（请求进行中时置灰，防连点）
    property bool okEnabled: true
    /// 「取消」能不能点（请求进行中时也禁用，避免界面说「已取消」但请求还在跑）
    property bool cancelEnabled: true
    /// 「确定」的文案
    property string okText: qsTr("确定")

    signal okClicked()
    signal cancelClicked()

    spacing: 8

    Item {
        Layout.fillWidth: true
    }

    Button {
        objectName: "dialogFooterCancel"
        text: qsTr("取消")
        enabled: footer.cancelEnabled
        onClicked: footer.cancelClicked()
    }

    Button {
        objectName: "dialogFooterOk"
        text: footer.okText
        enabled: footer.okEnabled
        onClicked: footer.okClicked()
    }
}
