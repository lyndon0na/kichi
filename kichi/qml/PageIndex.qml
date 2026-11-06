import QtQuick

// 侧栏、页头和页面栈共用同一组索引；新增页面时只改这里。
QtObject {
    readonly property int files: 0
    readonly property int shares: 1
    // 传输任务页同时也是任务中心：离线任务作为页内标签，不再单独占页面索引。
    readonly property int transfers: 2
    readonly property int trash: 3
    // 设置是独立窗口，不占页面索引与页面栈
    readonly property int recentSaved: 4
    readonly property int playbackHistory: 5
    readonly property int starred: 6

    readonly property var libraryPages: [recentSaved, playbackHistory, starred]
}
