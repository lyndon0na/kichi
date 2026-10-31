import QtQuick

// 最近保存页：上传与转存事件共用资料库表格。
LibraryPage {
    kind: "recent-saved"
    primaryHeader: qsTr("大小")
    secondaryHeader: qsTr("保存时间")
    emptyText: qsTr("还没有最近保存的文件")
}
