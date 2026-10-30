import QtQuick

// 紧凑的三条均衡器动画。只在「播放中」状态可见时运行，避免列表里
// 几十个隐藏委托各自驱动无限动画。
Item {
    id: root

    property color fillColor: "#ffffff"
    property bool running: true

    implicitWidth: 10
    implicitHeight: 10

    Rectangle {
        id: leftBar
        objectName: "playingBarLeft"

        x: 0
        width: 2
        height: 4
        radius: 1
        anchors.bottom: parent.bottom
        color: root.fillColor
    }

    Rectangle {
        id: middleBar
        objectName: "playingBarMiddle"

        x: 4
        width: 2
        height: 7
        radius: 1
        anchors.bottom: parent.bottom
        color: root.fillColor
    }

    Rectangle {
        id: rightBar
        objectName: "playingBarRight"

        x: 8
        width: 2
        height: 5
        radius: 1
        anchors.bottom: parent.bottom
        color: root.fillColor
    }

    SequentialAnimation {
        loops: Animation.Infinite
        running: root.running && root.visible

        NumberAnimation {
            target: leftBar
            property: "height"
            to: 9
            duration: 360
            easing.type: Easing.InOutSine
        }
        NumberAnimation {
            target: leftBar
            property: "height"
            to: 3
            duration: 300
            easing.type: Easing.InOutSine
        }
    }

    SequentialAnimation {
        loops: Animation.Infinite
        running: root.running && root.visible

        NumberAnimation {
            target: middleBar
            property: "height"
            to: 3
            duration: 320
            easing.type: Easing.InOutSine
        }
        NumberAnimation {
            target: middleBar
            property: "height"
            to: 10
            duration: 380
            easing.type: Easing.InOutSine
        }
    }

    SequentialAnimation {
        loops: Animation.Infinite
        running: root.running && root.visible

        NumberAnimation {
            target: rightBar
            property: "height"
            to: 8
            duration: 420
            easing.type: Easing.InOutSine
        }
        NumberAnimation {
            target: rightBar
            property: "height"
            to: 3
            duration: 280
            easing.type: Easing.InOutSine
        }
    }
}
