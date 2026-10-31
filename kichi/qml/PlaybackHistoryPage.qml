import QtQuick

// 播放历史页：共用资料库表格，数据来自官方 TYPE_PLAY 事件。
LibraryPage {
    kind: "playback-history"
    primaryHeader: qsTr("播放进度")
    secondaryHeader: qsTr("最后播放")
    emptyText: qsTr("还没有播放记录")
}
