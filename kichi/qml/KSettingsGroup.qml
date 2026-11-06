import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 设置页分组：一个小小的分组标题 + 一组直接铺在面板上的设置行。
// 卡片式设置不再给每组套卡片，行与行之间也不画分隔线，只靠留白分组；
// 组与组之间的间距由页面控制。
ColumnLayout {
    id: group

    property string title: ""
    default property alias content: body.data

    readonly property KTheme theme: KTheme { parent: group }

    spacing: theme.gapSm

    KSectionTitle {
        text: group.title
        Layout.fillWidth: true
    }

    ColumnLayout {
        id: body
        Layout.fillWidth: true
        spacing: 0
    }
}
