import QtQuick
import QtQuick.Controls
import "../components"
import "../core"

// 播放浮层里的一行：白字 + 悬停白色淡底 + 选中白色勾。
// 不用 MenuItem，避免系统风格把复选框 / 内边距 / 底色带进来。
//
// `badge` 是左侧的小标签（连播队列用它显示集数或 `#N`）；轨道菜单留空。
Item {
    id: row

    property string text: ""
    property string badge: ""
    /// 队列位置（1 起）；只有集数和位置不一致时才显示在右侧。
    property int position: 0
    property bool showPosition: false
    property bool selected: false
    property bool interactive: true
    /// 非空时在右侧画这个图标（例如「字幕样式…」的 chevron-right）
    property string trailingIcon: ""
    signal clicked()

    readonly property KTheme theme: KTheme { parent: row }

    implicitWidth: 100
    implicitHeight: 34

    Rectangle {
        anchors.fill: parent
        anchors.margins: 1
        radius: 6
        color: !row.interactive ? "transparent"
             : mouse.pressed ? theme.mediaPressed
             : row.selected ? theme.mediaSelected
             : mouse.containsMouse ? theme.mediaHover
             : "transparent"
    }

    // 左侧集数标签：`02` / `S01E10` / `第2集`，或解析不出来时的 `#N`。
    Rectangle {
        id: badgeBox

        visible: row.badge.length > 0
        anchors.left: parent.left
        anchors.leftMargin: 7
        anchors.verticalCenter: parent.verticalCenter
        width: badgeLabel.implicitWidth + 10
        height: 20
        radius: 5
        color: row.selected ? "#33FFFFFF" : "#26FFFFFF"

        Label {
            id: badgeLabel

            anchors.centerIn: parent
            text: row.badge
            color: theme.mediaText
            font.pixelSize: 11
            font.weight: Font.Medium
        }
    }

    Label {
        id: label

        anchors.left: badgeBox.visible ? badgeBox.right : parent.left
        anchors.leftMargin: badgeBox.visible ? 8 : 10
        anchors.right: positionLabel.visible ? positionLabel.left : icon.left
        anchors.rightMargin: 8
        anchors.verticalCenter: parent.verticalCenter
        text: row.text
        textFormat: Text.PlainText
        elide: Text.ElideRight
        color: theme.mediaText
        opacity: row.interactive ? 1 : 0.5
        font.pixelSize: 13
    }

    // 集数与队列位置不一致时的补充信息：很淡的 `#N`，不抢集数主标签。
    Label {
        id: positionLabel

        visible: row.showPosition && row.position > 0
        anchors.right: icon.left
        anchors.rightMargin: 8
        anchors.verticalCenter: parent.verticalCenter
        text: "#" + row.position
        color: theme.mediaText
        opacity: 0.4
        font.pixelSize: 11
    }

    NavIcon {
        id: icon

        visible: row.selected || row.trailingIcon.length > 0
        anchors.right: parent.right
        anchors.rightMargin: 10
        anchors.verticalCenter: parent.verticalCenter
        width: 16
        height: 16
        name: row.selected ? "check" : row.trailingIcon
        color: theme.mediaText
        opacity: row.selected ? 1 : 0.75
    }

    // 文件名很长时，悬停看完整名字（集数标签已经单独显示，不再靠猜）。
    KMediaToolTip {
        visible: mouse.containsMouse && label.implicitWidth > label.width
        text: row.text
    }

    MouseArea {
        id: mouse

        anchors.fill: parent
        hoverEnabled: true
        enabled: row.interactive
        cursorShape: row.interactive ? Qt.PointingHandCursor : Qt.ArrowCursor
        onClicked: row.clicked()
    }
}
