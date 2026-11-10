import QtQuick

// 列表行装饰：左侧 3px 强调条。
//
// 只画这一根线；行的背景色 / 圆角 / hover 优先级留给各行自己。
// 强调条内缩量是各页刻意的差异（任务行 20、传输行 14、其余 28），
// 由调用方通过 accentInset 指定，不在这里统一。
Item {
    id: chrome

    property bool accentVisible: false
    property int accentInset: 28
    property string accentObjectName: ""
    property color accentColor: "transparent"

    Rectangle {
        objectName: chrome.accentObjectName
        visible: chrome.accentVisible
        width: 3
        height: Math.max(0, chrome.height - chrome.accentInset)
        radius: 1.5
        anchors.left: parent.left
        anchors.leftMargin: 1
        anchors.verticalCenter: parent.verticalCenter
        color: chrome.accentColor
    }
}
