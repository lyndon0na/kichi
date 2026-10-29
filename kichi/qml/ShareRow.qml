import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 分享管理页的一条卡片。选择走显式复选框，批量撤销的命中范围始终可见。
ItemDelegate {
    id: row

    required property string key
    required property string title
    required property string url
    required property string passCode
    required property bool hasPassword
    required property string accessText
    required property string expirationText
    required property string createdText
    required property string statsText
    required property string statusText
    required property string statusTone
    property bool selected: false
    property bool actionRunning: false

    signal selectionToggled(bool checked)
    signal copyLinkRequested()
    signal copyPasswordRequested()
    signal revokeRequested()

    KTheme { id: theme }

    // 右侧三个 30px 操作按钮 + 间距 + 内容上下留白。
    implicitHeight: 116
    leftPadding: 12
    rightPadding: 12
    topPadding: 10
    bottomPadding: 10

    background: Rectangle {
        radius: theme.radiusCard
        color: row.selected ? theme.accentSoft : theme.surfaceRaised
        border.width: 1
        border.color: row.selected ? theme.softTint(theme.accent, 0.36)
                                   : theme.hairline

        Rectangle {
            visible: row.selected
            width: 3
            height: parent.height - 18
            radius: 1.5
            anchors.left: parent.left
            anchors.leftMargin: 1
            anchors.verticalCenter: parent.verticalCenter
            color: theme.accent
        }
    }

    contentItem: RowLayout {
        spacing: theme.gapMd

        CheckBox {
            objectName: "shareSelectCheckBox"
            checked: row.selected
            Accessible.name: qsTr("选择分享「%1」").arg(row.title)
            Layout.alignment: Qt.AlignTop
            onToggled: row.selectionToggled(checked)
        }

        ColumnLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 5

            RowLayout {
                Layout.fillWidth: true
                spacing: 8

                Label {
                    Layout.fillWidth: true
                    text: row.title
                    font.pixelSize: theme.fontBody
                    font.weight: Font.DemiBold
                    color: theme.textPrimary
                    elide: Text.ElideRight
                    verticalAlignment: Text.AlignVCenter
                }

                KStatusBadge {
                    text: row.statusText
                    tone: row.statusTone
                }

                KStatusBadge {
                    text: row.accessText
                    tone: row.hasPassword ? "warning" : "neutral"
                }
            }

            Label {
                Layout.fillWidth: true
                text: row.url
                font.pixelSize: theme.fontMeta
                color: theme.textMuted
                elide: Text.ElideMiddle
                verticalAlignment: Text.AlignVCenter
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: theme.gapSm

                Label {
                    Layout.fillWidth: true
                    text: {
                        const created = row.createdText.length > 0
                                        ? qsTr("创建于 %1").arg(row.createdText) : ""
                        return [created, row.expirationText, row.statsText]
                            .filter(function (part) { return part.length > 0 }).join(" · ")
                    }
                    font.pixelSize: theme.fontMeta
                    color: theme.textMuted
                    elide: Text.ElideRight
                    verticalAlignment: Text.AlignVCenter
                }
            }
        }

        ColumnLayout {
            Layout.preferredWidth: 112
            Layout.alignment: Qt.AlignVCenter
            spacing: 3

            KButton {
                Layout.fillWidth: true
                implicitHeight: 30
                text: qsTr("复制链接")
                variant: "ghost"
                enabled: row.url.length > 0
                onClicked: row.copyLinkRequested()
            }

            KButton {
                Layout.fillWidth: true
                implicitHeight: 30
                visible: row.hasPassword
                text: qsTr("复制密码")
                variant: "ghost"
                enabled: row.passCode.length > 0
                onClicked: row.copyPasswordRequested()
            }

            KButton {
                Layout.fillWidth: true
                implicitHeight: 30
                text: qsTr("撤销")
                variant: "dangerOutline"
                enabled: !row.actionRunning
                onClicked: row.revokeRequested()
            }
        }
    }
}
