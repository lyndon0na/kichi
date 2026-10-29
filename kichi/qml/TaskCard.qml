import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 离线任务列表的一张卡片（从 TasksPage.qml 拆出来，守 QML 行数提醒线）。
//
// 视觉：浅一档的表面 + 极浅描边；进行中的卡片叠一层淡强调色并带左侧竖条，
// 与已完成 / 失败的任务在列表里一眼分开。状态文字用 KStatusBadge。
Frame {
    id: card

    // —— 数据（来自 TasksPage 的 ListModel，字段与 Rust task_map 一致）——
    required property string key
    required property string name
    required property string phase
    required property string phaseText
    required property int progress
    required property string progressText
    required property string sizeText
    required property string createdText
    required property string detail
    required property bool settled

    /// 点「删除」：由页面记下 key / name 再弹二次确认
    signal deleteRequested(string key, string name)

    KTheme { id: theme }

    padding: 14

    readonly property bool isActive: !card.settled

    /// 状态文字 → 徽标色调
    readonly property string tone: {
        if (card.phase === "error")
            return "danger"
        if (card.phase === "completed")
            return "success"
        if (card.phase === "pending" || card.phase === "running")
            return "accent"
        return "neutral"
    }

    background: Rectangle {
        radius: theme.radiusCard
        color: card.isActive
               ? Qt.tint(theme.surfaceRaised, theme.accentSoft)
               : theme.surfaceRaised
        border.width: 1
        border.color: card.isActive
                      ? theme.softTint(theme.accent, 0.30)
                      : theme.hairline

        Rectangle {
            visible: card.isActive
            width: 3
            height: parent.height - 20
            radius: 1.5
            anchors.left: parent.left
            anchors.leftMargin: 1
            anchors.verticalCenter: parent.verticalCenter
            color: theme.accent
        }
    }

    // ⚠ 必须显式给 contentItem：Frame 的默认 contentItem 只在「恰好一个子项」时
    // 才把这个子项当内容，本卡片里还有 `KTheme { }` 这个不可见兄弟项，于是
    // 默认 contentItem 谁都不认、高度停在 0——整张卡片塌成一条 28px 的空皮
    // （2026-10-29 用户截图：离线任务列表只剩一排绿色细条）。
    // 显式 contentItem 不会自动吃到 padding，内边距要自己给。
    contentItem: ColumnLayout {
        anchors.fill: parent
        anchors.margins: card.padding
        spacing: theme.gapSm

        RowLayout {
            Layout.fillWidth: true
            spacing: 10

            Label {
                Layout.fillWidth: true
                text: card.name
                elide: Text.ElideMiddle
                font.pixelSize: theme.fontSection
                font.weight: Font.DemiBold
                color: theme.textPrimary
            }

            KStatusBadge {
                text: card.phaseText
                tone: card.tone
            }

            KButton {
                text: qsTr("删除")
                variant: "ghost"
                implicitHeight: 26
                leftPadding: 10
                rightPadding: 10
                onClicked: card.deleteRequested(card.key, card.name)
            }
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: 10
            visible: card.isActive || card.phase === "error"

            KProgressBar {
                Layout.fillWidth: true
                from: 0
                to: 100
                value: card.progress
                indeterminate: card.phase === "pending"
            }

            Label {
                text: card.progressText
                font.pixelSize: theme.fontMeta
                color: theme.textMuted
            }
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: theme.gapMd

            Label {
                text: card.sizeText
                font.pixelSize: theme.fontMeta
                color: theme.textMuted
            }

            Label {
                text: card.createdText
                font.pixelSize: theme.fontMeta
                color: theme.textMuted
            }

            Label {
                Layout.fillWidth: true
                text: card.detail
                font.pixelSize: theme.fontMeta
                elide: Text.ElideRight
                color: card.phase === "error" ? theme.danger : theme.textMuted
            }
        }
    }
}
