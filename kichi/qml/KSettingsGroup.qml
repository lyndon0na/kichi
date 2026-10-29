import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 设置页分组：标题在卡片外，卡片只负责把同一组设置收在一起。
// 行内容由调用方通过默认属性放进卡片；组与组之间由页面控制间距。
ColumnLayout {
    id: group

    property string title: ""
    default property alias content: body.data

    readonly property KTheme theme: KTheme { }

    spacing: theme.gapSm

    KSectionTitle {
        text: group.title
        Layout.fillWidth: true
    }

    Rectangle {
        Layout.fillWidth: true
        implicitHeight: body.implicitHeight + 2
        color: theme.surfaceRaised
        radius: theme.radiusCard
        border.width: 1
        border.color: theme.hairline

        ColumnLayout {
            id: body
            anchors.fill: parent
            anchors.margins: 1
            spacing: 0
        }
    }
}
