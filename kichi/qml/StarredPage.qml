import QtQuick

// 已加星标页：公共骨架，星标列表由独立分支接入。
LibraryPage {
    kind: "starred"
    primaryHeader: qsTr("大小")
    secondaryHeader: qsTr("更新时间")
    emptyText: qsTr("星标数据将在后续分支接入")
}
