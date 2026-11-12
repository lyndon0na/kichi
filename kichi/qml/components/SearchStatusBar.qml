import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "../core"

// 搜索状态行（输入阶段只筛当前目录 / 全盘结果共用）。结果列表本身复用
// 文件页的 ListView / GridView，
// 这里只显示扫描进度、筛选入口 / 摘要、截断提示和停止按钮。
RowLayout {
    id: root

    property var backend: null
    property string statusText: backend ? backend.searchStatus : ""
    property bool truncated: backend !== null && backend.searchTruncated === true

    // 筛选变化只改 Rust 状态；这里发信号让文件页复用搜索框的防抖再跑一次。
    signal filterChanged()

    // 只有网络全盘扫描才转圈 / 提供停止；本地索引搜索不显示这些
    readonly property bool scanning: backend !== null
                                     && backend.searchActive === true
                                     && backend.searchSettled !== true
                                     && backend.searchSource === "network"
    // 旧测试替身可能没有筛选属性，按「没有筛选」处理。
    readonly property int activeFilterCount: backend !== null
                                             && typeof backend.searchFilterActiveCount !== "undefined"
                                             ? backend.searchFilterActiveCount : 0
    readonly property string filterSummary: backend !== null
                                            && typeof backend.searchFilterSummary !== "undefined"
                                            ? (backend.searchFilterSummary || "") : ""
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

    // 筛选摘要：条件和「筛选」按钮上的数字同源，避免两处口径分叉。
    ElidedLabel {
        objectName: "searchFilterSummary"
        visible: root.activeFilterCount > 0 && root.filterSummary.length > 0
        Layout.maximumWidth: 260
        text: qsTr("筛选：%1").arg(root.filterSummary)
        elide: Text.ElideRight
        maximumLineCount: 1
        color: palette.placeholderText
        font.pixelSize: 12
    }

    Label {
        visible: root.truncated
        text: qsTr("（结果只显示前 500 条，命中数见左）")
        color: palette.placeholderText
        font.pixelSize: 12
    }

    SearchFilterButton {
        backend: root.backend
        Layout.alignment: Qt.AlignVCenter
        onFilterChanged: root.filterChanged()
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
