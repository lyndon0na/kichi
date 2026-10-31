import QtQuick

// 播放历史页：公共骨架，查询与进度列由独立分支接入。
LibraryPage {
    kind: "playback-history"
    primaryHeader: qsTr("播放进度")
    secondaryHeader: qsTr("最后播放")
    emptyText: qsTr("播放历史的数据将在后续分支接入")
}
