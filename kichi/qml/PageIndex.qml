import QtQuick

// 侧栏、页头和页面栈共用同一组索引；新增页面时只改这里。
QtObject {
    readonly property int files: 0
    readonly property int shares: 1
    readonly property int transfers: 2
    readonly property int tasks: 3
    readonly property int trash: 4
    readonly property int settings: 5
    readonly property int recentSaved: 6
    readonly property int playbackHistory: 7
    readonly property int starred: 8

    readonly property var libraryPages: [recentSaved, playbackHistory, starred]
}
