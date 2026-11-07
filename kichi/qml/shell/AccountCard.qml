import QtQuick
import QtQuick.Controls
import QtQuick.Effects
import QtQuick.Layouts
import "../components"
import "../core"

// 侧栏底部的账号区（从 SideBar.qml 拆出来，守 QML 行数提醒线）。
//
// 一行：头像 + 用户名 + 右侧设置入口。容量已挪到底栏右侧（QuotaIndicator.qml）；
// 会员标在 2026-11-05 去掉——身份只留头像和名字，不再和设置按钮抢位置。
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
    /// 当前停在设置页：右侧设置按钮点亮强调色
    property bool settingsActive: false

    signal settingsRequested()

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

    /// 骨架条的呼吸透明度（只在加载中动）
    property real skeletonPulse: 1

    // 账号信息是侧栏页脚，不再做成独立卡片：只用一条淡分隔线收尾，
    // 避免和导航选中项争夺视觉层级。
    // 自己不再留底部内边距：侧栏本身有 12px 外边距补齐，否则
    // 「分隔线→账号行」比「账号行→窗口底」少一截，整行看着偏上（2026-11-07 用户反馈）。
    implicitHeight: layout.y + layout.implicitHeight

    Rectangle {
        objectName: "accountDivider"
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
                    objectName: "avatarImage"
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
                    // HACK(Qt 6.11.2): DPR 遍历期间重载图片会改变 visible，
                    // 同步销毁 layer.effect 会让 Qt 继续访问已释放的遮罩。
                    layer.enabled: true
                    layer.effect: MultiEffect {
                        objectName: "avatarMaskEffect"
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

            // 设置是页面，但常驻入口放在账号行右侧：不放 checkable，避免
            // 点击改写 checked、永久切断与 settingsActive 的绑定（同侧栏导航项）。
            KIconButton {
                objectName: "accountSettingsButton"
                Layout.alignment: Qt.AlignVCenter
                implicitWidth: 28
                implicitHeight: 28
                iconName: "settings"
                iconSize: 16
                iconOpacity: 0.7
                checked: card.settingsActive

                Accessible.name: qsTr("设置")
                ToolTip.visible: hovered
                ToolTip.text: qsTr("设置（Ctrl+,）")
                onClicked: card.settingsRequested()
            }
        }
    }

    // 骨架的呼吸动画：只动透明度，不画实体色块，安静地表示「正在取」
    SequentialAnimation {
        running: card.identityPending
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
