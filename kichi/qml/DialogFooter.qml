pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 弹窗页脚：左「确定」、右「取消」（异步操作的弹窗共用；顺序与系统标准按钮一致，
// 两套页脚的「最右一个」都是取消，同一个位置做的事相同）。
//
// 为什么不用 `standardButtons`：QQC2 的 Dialog 点在 Ok（AcceptRole）上会自己
// `close()`，而这两个操作都是异步请求——失败原因回来时对话框已经不可见，
// 用户只看到「点了确定就消失了」。页脚按钮只发信号，由弹窗自己决定什么时候关：
// 成功收起，失败停在原地显示原因（见 `FilesPageMenu.qml` 的 onFileActionDone）。
//
// ⚠ Dialog 的 `footer` 是**整宽、贴底**摆放的，不吃 Dialog.padding
//   （2026-10-28 实测：footer.x = 0、footer.width = dialog.width、
//   footer 底边与弹窗底边齐平），所以按钮会顶到弹窗右下角。这里自己留边距，
//   取值传所在 Dialog 的 padding，和正文的左右留白对齐。
Item {
    id: footer

    /// 「确定」能不能点（请求进行中时置灰，防连点）
    property bool okEnabled: true
    /// 「取消」能不能点（请求进行中时也禁用，避免界面说「已取消」但请求还在跑）
    property bool cancelEnabled: true
    /// 「确定」的文案
    property string okText: qsTr("确定")
    /// 按钮离弹窗左 / 右 / 下边缘的距离：传所在 Dialog 的 `padding`
    property int outerMargin: 12

    signal okClicked()
    signal cancelClicked()

    // 上边距交给 Dialog 的 spacing（正文和页脚之间本就有间隔），这里只补三边
    implicitWidth: row.implicitWidth + footer.outerMargin * 2
    implicitHeight: row.implicitHeight + footer.outerMargin

    RowLayout {
        id: row

        anchors.fill: parent
        anchors.leftMargin: footer.outerMargin
        anchors.rightMargin: footer.outerMargin
        anchors.bottomMargin: footer.outerMargin

        spacing: 8

        Item {
            Layout.fillWidth: true
        }

        // 顺序跟系统标准按钮一致：确定在左、取消在右（KDE 默认）。
        // 两套页脚的「最右一个」都是取消，避免同一个位置在两类弹窗里做不同的事。
        KButton {
            objectName: "dialogFooterOk"
            text: footer.okText
            variant: "primary"
            enabled: footer.okEnabled
            onClicked: footer.okClicked()
        }

        KButton {
            objectName: "dialogFooterCancel"
            text: qsTr("取消")
            variant: "secondary"
            enabled: footer.cancelEnabled
            onClicked: footer.cancelClicked()
        }
    }
}
