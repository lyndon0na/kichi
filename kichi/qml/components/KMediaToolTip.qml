import QtQuick
import QtQuick.Controls

// 播放窗口的深色提示气泡：和轨道菜单 / 面板同一套表面，不再用系统浅色 ToolTip。
// 颜色是刻意的字面量——视频界面固定深色，不能跟浅色桌面主题走。
ToolTip {
    id: tip

    delay: 600
    timeout: 4000
    padding: 7

    background: Rectangle {
        color: "#F0101010"
        radius: 6
        border.width: 1
        border.color: "#26FFFFFF"
    }

    contentItem: Label {
        text: tip.text
        color: "#F2FFFFFF"
        font.pixelSize: 12
    }
}
