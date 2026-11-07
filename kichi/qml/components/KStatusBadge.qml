import QtQuick
import QtQuick.Controls
import "../core"

// 状态药丸：把「已完成 / 失败 / 下载中」这类状态文字包进一枚淡色圆角标签，
// 比裸文字更容易在卡片扫读时一眼分辨。
//
// tone：neutral | accent | success | warning | danger
// 颜色仍然全部来自 KTheme（系统强调色 + 状态色），不写死具体色值。
Rectangle {
    id: badge

    property string text: ""
    property string tone: "neutral"

    KTheme { id: theme }

    readonly property color toneColor: {
        if (badge.tone === "success")
            return theme.success
        if (badge.tone === "warning")
            return theme.warning
        if (badge.tone === "danger")
            return theme.danger
        if (badge.tone === "accent")
            return theme.accent
        return theme.textMuted
    }

    implicitWidth: label.implicitWidth + 14
    implicitHeight: 20
    // 小圆角而不是满圆：KDE 控件都是 5px 级别的圆角，
    // 药丸状在这里会显得比周围控件「软」一档
    radius: theme.radiusSmall
    color: theme.softTint(toneColor, badge.tone === "neutral" ? 0.10 : 0.14)

    Label {
        id: label
        anchors.centerIn: parent
        text: badge.text
        font.pixelSize: theme.fontCaption
        font.weight: Font.DemiBold
        color: badge.toneColor
    }
}
