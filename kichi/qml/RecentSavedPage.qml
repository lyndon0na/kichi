import QtQuick

// 最近保存页：公共骨架，查询实现由独立分支接入。
LibraryPage {
    kind: "recent-saved"
    primaryHeader: qsTr("大小")
    secondaryHeader: qsTr("保存时间")
    emptyText: qsTr("最近保存的数据将在后续分支接入")
}
