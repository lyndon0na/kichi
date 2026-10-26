import QtQuick
import QtQuick.Controls

// 清晰度菜单：后端取到档位后 playMenu 非空即弹出。
//
// 从 FilesPage.qml 拆出来：那是页面结构，这里是播放相关的弹层。
// 页面只需要在拿到「▶」按钮位置时调 openAt()。
Menu {
    id: menu

    property var backend: null
    // 弹出锚点（页面坐标系，通常是「▶」按钮下方中点）
    property point anchorPoint: Qt.point(0, 0)

    // 弹出时按锚点定位并夹在父项范围内
    function openAt(point) {
        anchorPoint = point
        x = Math.max(0, Math.min(point.x - width / 2, (menu.parent ? menu.parent.width : 0) - width))
        y = Math.max(0, Math.min(point.y + 4, (menu.parent ? menu.parent.height : 0) - height))
        open()
    }

    // 用户点菜单外面关掉时，把后端的档位一起清掉（避免残留的直链）
    onClosed: menu.backend ? menu.backend.cancelPlay() : undefined

    // 标题 + 档位条目（不用 contentItem：老 Qt 版本没有这个属性）
    MenuItem {
        text: menu.backend ? menu.backend.playTitle : ""
        enabled: false
        width: 260
    }

    MenuSeparator {}

    Repeater {
        model: menu.backend ? menu.backend.playMenu : []

        delegate: MenuItem {
            text: modelData.label
                  + (modelData.reason.length > 0 ? qsTr("（%1）").arg(modelData.reason) : "")
                  + (modelData.original ? qsTr(" · 原文件") : "")
            enabled: modelData.available === true
            width: 260
            onClicked: menu.backend.playSelected(index)
        }
    }

    // playMenu 一有内容就弹（后端已经过滤了「全部不可用」的情况）
    Connections {
        target: menu.backend

        function onChanged() {
            // 后端清空了菜单（选中开播 / 出错 / 登出），界面上的菜单也要收起
            if (!menu.backend || menu.backend.playMenu.length === 0) {
                if (menu.visible)
                    menu.close()
                return
            }
            if (!menu.visible)
                menu.openAt(menu.anchorPoint)
        }
    }
}
