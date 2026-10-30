import QtQuick
import QtQuick.Controls
import QtQuick.Effects
import QtQuick.Layouts

// 侧栏底部的账号区（从 SideBar.qml 拆出来，守 QML 行数提醒线）。
//
// 第一行：头像 + 用户名 + 会员标。
// 第二行：已用容量 / 上限 + 容量条。
//
// 退出登录不在这里：它是低频、不可逆的操作，放在设置页「账号」一节，
// 点下去还要过一道确认（2026-10-29 用户要求，别再加回账号区）。
//
// 账号信息要等一次网络往返才回来：这段时间画骨架（灰条呼吸），区域高度固定，
// 信息到达时原地替换、不抽动。**不要退成「已登录」这种假占位**——先说「已登录」
// 再跳出真名，同一块位置两次跳变，很别扭（2026-10-29 用户反馈）。
Item {
    id: card

    /// 账号（邮箱）；空串 = 还在取，画骨架
    property string account: ""
    /// `/v1/user/me` 的昵称；优先于账号本地部分显示
    property string displayName: ""
    /// Rust 下载后的本地头像 `file://` URI；空串时显示首字母
    property string avatarPath: ""
    property string quotaUsed: ""
    property string quotaLimit: ""
    /// 容量请求在途；与昵称加载分开，缓存昵称不会误伤容量骨架
    property bool quotaLoading: false
    /// < 0 表示不限量 / 未知，此时不画容量条
    property double quotaRatio: -1
    property bool vip: false

    readonly property KTheme theme: KTheme { parent: card }

    /// 没有昵称时，用账号 @ 前半段兜底。
    readonly property string accountName: {
        if (card.account.length === 0)
            return ""
        var at = card.account.indexOf("@")
        return at > 0 ? card.account.substring(0, at) : card.account
    }

    readonly property string shownName: {
        var name = card.displayName.trim()
        return name.length > 0 ? name : card.accountName
    }

    readonly property bool identityPending: card.shownName.length === 0
    readonly property bool quotaPending: card.quotaLoading
                                          && card.quotaUsed.length === 0
                                          && card.quotaLimit.length === 0

    /// 骨架条的呼吸透明度（只在加载中动）
    property real skeletonPulse: 1

    // 账号信息是侧栏页脚，不再做成独立卡片：只用一条淡分隔线收尾，
    // 避免和导航选中项争夺视觉层级。
    implicitHeight: layout.y + layout.implicitHeight + theme.gapSm

    Rectangle {
        width: parent.width
        height: 1
        color: theme.divider
    }

    ColumnLayout {
        id: layout
        x: theme.gapMd
        y: theme.gapMd + 1
        width: parent.width - 2 * theme.gapMd
        spacing: theme.gapSm

        // —— 第一行：头像 + 用户名 + 会员标 ——
        RowLayout {
            Layout.fillWidth: true
            spacing: theme.gapSm

            // 头像：有本地图片就裁成圆形，否则显示强调色首字母。
            Rectangle {
                objectName: "avatarCircle"
                Layout.alignment: Qt.AlignVCenter
                implicitWidth: 24
                implicitHeight: 24
                radius: 12
                color: theme.accentGhost
                opacity: card.identityPending ? card.skeletonPulse : 1

                Label {
                    objectName: "avatarInitial"
                    anchors.centerIn: parent
                    visible: !card.identityPending && avatarImage.status !== Image.Ready
                    text: card.shownName.length > 0
                          ? card.shownName.charAt(0).toUpperCase() : ""
                    color: theme.accent
                    font.pixelSize: theme.fontMeta
                    font.weight: Font.Bold
                }

                Image {
                    id: avatarImage
                    anchors.fill: parent
                    source: card.avatarPath
                    visible: status === Image.Ready
                    sourceSize.width: 48
                    sourceSize.height: 48
                    fillMode: Image.PreserveAspectCrop
                    asynchronous: true
                    cache: false
                    smooth: true
                    mipmap: true
                    layer.enabled: visible
                    layer.effect: MultiEffect {
                        maskEnabled: true
                        maskSource: avatarMask
                    }
                }

                Item {
                    id: avatarMask
                    width: avatarImage.width
                    height: avatarImage.height
                    visible: false
                    layer.enabled: true

                    Rectangle {
                        anchors.fill: parent
                        radius: width / 2
                        color: "white"
                    }
                }
            }

            ElidedLabel {
                Layout.fillWidth: true
                visible: !card.identityPending
                text: card.shownName
                color: theme.textPrimary
                font.pixelSize: theme.fontMeta
                font.weight: Font.DemiBold
                elide: Text.ElideRight
                toolTipText: card.account.length > 0 ? card.account : card.shownName
                showToolTipWhenUntruncated: true
            }

            // 骨架：占住用户名这一行的宽度，信息到达后原地换成文字
            Rectangle {
                Layout.fillWidth: true
                Layout.preferredHeight: 10
                visible: card.identityPending
                radius: 2
                color: theme.surfacePressed
                opacity: card.skeletonPulse
            }

            Rectangle {
                objectName: "memberBadge"
                Layout.alignment: Qt.AlignVCenter
                visible: !card.identityPending && card.vip
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
        // 真实文字和骨架共用同一个 16px 槽；容量回来时只换内容，不改变高度。
        Item {
            objectName: "quotaTextSlot"
            Layout.fillWidth: true
            Layout.preferredHeight: 16

            ElidedLabel {
                anchors.fill: parent
                visible: !card.quotaPending && card.quotaLimit.length > 0
                text: qsTr("已用 %1 / %2").arg(card.quotaUsed).arg(card.quotaLimit)
                elide: Text.ElideRight
                verticalAlignment: Text.AlignVCenter
                color: theme.textMuted
                font.pixelSize: theme.fontCaption
                showToolTipWhenUntruncated: true
            }

            Rectangle {
                objectName: "quotaSkeleton"
                anchors.fill: parent
                readonly property bool showing: card.quotaPending
                visible: showing
                radius: 2
                color: theme.surfacePressed
                opacity: card.skeletonPulse
            }
        }

        // 容量条：quotaRatio < 0 表示不限量 / 未知，此时只留 3px 空位不画条。
        // 高度锁死为 3px，加载前后始终占位，避免容量数据到达时把账号区撑开。
        Rectangle {
            objectName: "quotaBar"
            Layout.fillWidth: true
            Layout.preferredHeight: 3
            Layout.minimumHeight: 3
            Layout.maximumHeight: 3
            radius: height / 2
            color: card.quotaPending ? theme.surfacePressed
                                     : card.quotaRatio >= 0 ? theme.surfacePressed : "transparent"
            opacity: card.quotaPending ? card.skeletonPulse : 1

            Rectangle {
                width: parent.width * Math.max(card.quotaRatio, 0)
                height: parent.height
                radius: parent.radius
                color: theme.accent
                visible: !card.quotaPending && card.quotaRatio >= 0
            }
        }
    }

    // 骨架的呼吸动画：只动透明度，不画实体色块，安静地表示「正在取」
    SequentialAnimation {
        running: card.identityPending || card.quotaPending
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
