pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 弹窗页脚：左侧可带状态提示，右侧依次「确定」「取消」（异步操作的弹窗共用；
// 顺序与系统标准按钮一致，两套页脚的「最右一个」都是取消）。
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
    /// 某些弹窗不需要取消 / 关闭按钮
    property bool cancelVisible: true
    /// 左侧状态提示；为空时只显示右侧按钮
    property string statusText: ""
    property color statusColor: "transparent"
    /// 「确定」的文案
    property string okText: qsTr("确定")
    /// 「确定」的按钮语义（危险操作传 danger）
    property string okVariant: "primary"
    property string okObjectName: "dialogFooterOk"
    /// 「取消」的文案
    property string cancelText: qsTr("取消")
    property string cancelVariant: "secondary"
    property string cancelObjectName: "dialogFooterCancel"
    /// 按钮离弹窗左 / 右 / 下边缘的距离：传所在 Dialog 的 `padding`
    property int outerMargin: 12

    signal okClicked()
    signal cancelClicked()

    KTheme { id: theme }

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

        ElidedLabel {
            objectName: "dialogFooterStatus"
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            visible: footer.statusText.length > 0
            text: footer.statusText
            color: footer.statusColor
            font.pixelSize: theme.fontMeta
            elide: Text.ElideRight
            verticalAlignment: Text.AlignVCenter
        }

        Item {
            Layout.fillWidth: true
            visible: footer.statusText.length === 0
        }

        // 顺序跟系统标准按钮一致：确定在左、取消在右（KDE 默认）。
        // 两套页脚的「最右一个」都是取消，避免同一个位置在两类弹窗里做不同的事。
        KButton {
            objectName: footer.okObjectName
            text: footer.okText
            variant: footer.okVariant
            enabled: footer.okEnabled
            onClicked: footer.okClicked()
        }

        KButton {
            objectName: footer.cancelObjectName
            text: footer.cancelText
            variant: footer.cancelVariant
            enabled: footer.cancelEnabled
            visible: footer.cancelVisible
            onClicked: footer.cancelClicked()
        }
    }
}
