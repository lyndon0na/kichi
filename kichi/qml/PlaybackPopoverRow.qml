import QtQuick
import QtQuick.Controls

// 播放浮层里的一行：白字 + 悬停白色淡底 + 选中白色勾。
// 不用 MenuItem，避免系统风格把复选框 / 内边距 / 底色带进来。
Item {
    id: row

    property string text: ""
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

    Label {
        anchors.left: parent.left
        anchors.leftMargin: 10
        anchors.right: icon.left
        anchors.rightMargin: 8
        anchors.verticalCenter: parent.verticalCenter
        text: row.text
        textFormat: Text.PlainText
        elide: Text.ElideRight
        color: theme.mediaText
        opacity: row.interactive ? 1 : 0.5
        font.pixelSize: 13
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

    MouseArea {
        id: mouse

        anchors.fill: parent
        hoverEnabled: true
        enabled: row.interactive
        cursorShape: row.interactive ? Qt.PointingHandCursor : Qt.ArrowCursor
        onClicked: row.clicked()
    }
}
