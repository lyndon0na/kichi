import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "../core"

// 全盘搜索的状态行。结果列表本身复用文件页的 ListView / GridView，
// 这里只显示扫描进度、截断提示和停止入口。
RowLayout {
    id: root

    property var backend: null
    property string statusText: backend ? backend.searchStatus : ""
    property bool truncated: backend !== null && backend.searchTruncated === true

    // 只有网络全盘扫描才转圈 / 提供停止；本地索引搜索不显示这些
    readonly property bool scanning: backend !== null
                                     && backend.searchActive === true
                                     && backend.searchSettled !== true
                                     && backend.searchSource === "network"
    readonly property KTheme theme: KTheme { parent: root }

    Layout.leftMargin: theme.gapMd
    Layout.rightMargin: theme.gapMd
    spacing: theme.gapSm

    BusyIndicator {
        running: root.scanning
        visible: running
        Layout.preferredWidth: 20
        Layout.preferredHeight: 20
    }

    ElidedLabel {
        objectName: "searchStatusText"
        Layout.fillWidth: true
        text: root.statusText
        wrapMode: Text.Wrap
        elide: Text.ElideRight
        maximumLineCount: 2
        color: palette.text
    }

    Label {
        visible: root.truncated
        text: qsTr("（结果只显示前 500 条，命中数见左）")
        color: palette.placeholderText
        font.pixelSize: 12
    }

    KButton {
        text: qsTr("停止")
        visible: root.scanning
        variant: "ghost"
        implicitHeight: 28
        onClicked: {
            if (root.backend)
                root.backend.stopSearch()
        }
    }
}
