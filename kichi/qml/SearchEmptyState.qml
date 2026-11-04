import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 全盘搜索结束且没有命中时的排查建议。结果视图本身由文件页复用。
ColumnLayout {
    id: root

    property var backend: null

    visible: backend !== null
             && backend.searchActive === true
             && backend.searchSettled === true
             && backend.searchResults.length === 0
    spacing: 8

    Label {
        Layout.fillWidth: true
        horizontalAlignment: Text.AlignHCenter
        text: qsTr("没有找到名称包含「%1」的条目")
            .arg(root.backend ? root.backend.searchQuery : "")
        wrapMode: Text.Wrap
    }

    Label {
        Layout.fillWidth: true
        text: qsTr("可以试试：\n· 换更短的关键词（按名字任意位置匹配，不支持 * ? 通配符）\n· 本地索引可能还没收录最新上传的文件\n· 只搜文件名，不搜文件内容；回收站里的条目也会列出")
        color: palette.placeholderText
        wrapMode: Text.Wrap
        font.pixelSize: 12
    }
}
