import QtQuick
import QtQuick.Controls

// 由 build.rs 的 QmlModule("io.kichi") 生成
import io.kichi 1.0

ApplicationWindow {
    id: root

    title: "kichi"
    width: 1000
    height: 640
    visible: true
    color: palette.window

    Kichi {
        id: kichi
    }

    Column {
        anchors.centerIn: parent
        spacing: 12

        Label {
            anchors.horizontalCenter: parent.horizontalCenter
            text: "kichi"
            font.pixelSize: 40
            font.bold: true
            color: palette.text
        }

        Label {
            anchors.horizontalCenter: parent.horizontalCenter
            text: qsTr("PikPak 客户端 · 骨架就绪（v%1）").arg(kichi.version)
            color: palette.placeholderText
        }
    }
}
