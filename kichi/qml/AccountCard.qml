import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 侧栏底部的账号卡（从 SideBar.qml 拆出来，守 QML 行数提醒线）。
//
// 第一行：头像 + 用户名 + 会员标。
// 第二行：已用容量 / 上限 + 容量条。
//
// 退出登录不在这里：它是低频、不可逆的操作，放在设置页「账号」一节，
// 点下去还要过一道确认（2026-10-29 用户要求，别再加回账号卡）。
//
// 账号信息要等一次网络往返才回来：这段时间画骨架（灰条呼吸），卡片高度固定，
// 信息到达时原地替换、不抽动。**不要退成「已登录」这种假占位**——先说「已登录」
// 再跳出真名，同一块位置两次跳变，很别扭（2026-10-29 用户反馈）。
Rectangle {
    id: card

    /// 账号（邮箱）；空串 = 还在取，画骨架
    property string account: ""
    property string quotaUsed: ""
    property string quotaLimit: ""
    /// < 0 表示不限量 / 未知，此时不画容量条
    property double quotaRatio: -1
    property bool vip: false

    KTheme { id: theme }

    /// 用户名：账号 @ 前半段
    readonly property string accountName: {
        if (card.account.length === 0)
            return ""
        var at = card.account.indexOf("@")
        return at > 0 ? card.account.substring(0, at) : card.account
    }

    readonly property bool pending: card.accountName.length === 0

    /// 骨架条的呼吸透明度（只在加载中动）
    property real skeletonPulse: 1

    // 高度由内容决定：骨架和真数据同高（都是两行），信息回来时不变
    implicitHeight: layout.implicitHeight + 2 * theme.gapMd
    radius: theme.radiusCard
    color: theme.surfaceRaised
    border.width: 1
    border.color: theme.hairline

    ColumnLayout {
        id: layout
        x: theme.gapMd
        y: theme.gapMd
        width: parent.width - 2 * theme.gapMd
        spacing: theme.gapSm

        // —— 第一行：头像 + 用户名 + 会员标 ——
        RowLayout {
            Layout.fillWidth: true
            spacing: theme.gapSm

            // 头像：真数据是首字母，加载中是空圆
            Rectangle {
                Layout.alignment: Qt.AlignVCenter
                implicitWidth: 26
                implicitHeight: 26
                radius: 13
                color: theme.accentGhost

                Label {
                    anchors.centerIn: parent
                    visible: !card.pending
                    text: card.accountName.length > 0
                          ? card.accountName.charAt(0).toUpperCase() : ""
                    color: theme.accent
                    font.pixelSize: theme.fontBody
                    font.weight: Font.Bold
                }
            }

            ElidedLabel {
                Layout.fillWidth: true
                visible: !card.pending
                text: card.accountName
                color: theme.textPrimary
                font.pixelSize: theme.fontMeta
                font.weight: Font.DemiBold
                elide: Text.ElideRight
                toolTipText: card.account
                showToolTipWhenUntruncated: true
            }

            // 骨架：占住用户名这一行的宽度，信息到达后原地换成文字
            Rectangle {
                Layout.fillWidth: true
                Layout.preferredHeight: 10
                visible: card.pending
                radius: 2
                color: theme.surfacePressed
                opacity: card.skeletonPulse
            }

            Rectangle {
                Layout.alignment: Qt.AlignVCenter
                visible: !card.pending && card.vip
                implicitWidth: vipLabel.implicitWidth + 10
                implicitHeight: 15
                radius: theme.radiusSmall
                color: theme.accentGhost

                Label {
                    id: vipLabel
                    anchors.centerIn: parent
                    text: qsTr("会员")
                    color: theme.accent
                    font.pixelSize: theme.fontCaption
                    font.weight: Font.DemiBold
                }
            }
        }

        // —— 第二行：已用容量 / 上限 ——
        ElidedLabel {
            Layout.fillWidth: true
            visible: !card.pending && card.quotaLimit.length > 0
            text: qsTr("已用 %1 / %2").arg(card.quotaUsed).arg(card.quotaLimit)
            elide: Text.ElideRight
            color: theme.textMuted
            font.pixelSize: theme.fontCaption
            showToolTipWhenUntruncated: true
        }

        // 骨架：占住容量文字那一行（真数据是 16px 高的 Label）
        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: 16
            visible: card.pending
            radius: 2
            color: theme.surfacePressed
            opacity: card.skeletonPulse
        }

        // 容量条：quotaRatio < 0 表示不限量 / 未知，此时只留 4px 空位不画条。
        // ⚠ 用颜色透明而不是 visible: false —— 隐藏会把这一行从布局里抽走，
        // 骨架和真数据的高度就对不上，信息到达时卡片会抽一下。
        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: 4
            radius: height / 2
            color: card.pending ? theme.surfacePressed
                                : card.quotaRatio >= 0 ? theme.surfacePressed : "transparent"
            opacity: card.pending ? card.skeletonPulse : 1

            Rectangle {
                width: parent.width * Math.max(card.quotaRatio, 0)
                height: parent.height
                radius: parent.radius
                color: theme.accent
                visible: !card.pending && card.quotaRatio >= 0
            }
        }
    }

    // 骨架的呼吸动画：只动透明度，不画实体色块，安静地表示「正在取」
    SequentialAnimation {
        running: card.pending
        loops: Animation.Infinite
        NumberAnimation {
            target: card
            property: "skeletonPulse"
            from: 0.45
            to: 1.0
            duration: 750
            easing.type: Easing.InOutSine
        }
        NumberAnimation {
            target: card
            property: "skeletonPulse"
            from: 1.0
            to: 0.45
            duration: 750
            easing.type: Easing.InOutSine
        }
    }
}
