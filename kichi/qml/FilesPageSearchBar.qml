import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 文件页的搜索条（从 FilesPage.qml 抽出来：那里已到行数软上限）。
//
// 进去：输入即筛当前目录（0 请求），回车同样；清空 = 回当前目录。
// 「⌕ 全盘」：遍历网盘——唯一常驻的全盘入口（扫描中由结果头换成「停止」）。
Item {
    id: bar

    property var backend: null

    // 关键词改了（输入 / 回车 / 清空）：由页面去调 backend
    signal queryEdited(string text)
    // 点了 ✕
    signal exitRequested()
    // 在搜索框里按了 Esc
    signal escapeRequested()
    // 点了「⌕ 全盘」
    signal searchAllRequested()

    // 对外暴露输入框，页面拿它做 `/` 聚焦与 Esc 清空
    property alias field: searchField
    property alias text: searchField.text

    implicitHeight: searchField.implicitHeight

    RowLayout {
        anchors.fill: parent
        spacing: 8

        TextField {
            id: searchField
            Layout.fillWidth: true
            placeholderText: qsTr("在文件里按名字搜索（先筛当前目录；Esc 清空 / 退出）")
            selectByMouse: true
            text: bar.backend ? bar.backend.searchQuery : ""
            onTextEdited: bar.queryEdited(text)
            onAccepted: bar.queryEdited(text)

            // Esc：有字先清空回当前目录，已空则退出搜索视图。
            // 注意：TextField 没有 onEscapePressed 属性，Esc 只能挂在 Keys 附加对象上。
            // TextField 自己会「消费」Esc（取消编辑），所以要在按下的最早阶段拦下，
            // 否则事件冒不到 Keys.onEscapePressed。
            Keys.onShortcutOverride: function (event) {
                if (event.key === Qt.Key_Escape)
                    event.accepted = true
            }
            Keys.onEscapePressed: bar.escapeRequested()
        }

        // 全盘搜索：常驻（不再按搜索状态显隐），停止由结果头负责
        Button {
            readonly property bool running: bar.backend !== null
                                            && bar.backend.searchActive
                                            && !bar.backend.searchSettled
            text: qsTr("全盘搜索")
            enabled: bar.backend !== null && !running && bar.text.trim().length > 0
            ToolTip.visible: hovered
            ToolTip.text: qsTr("遍历整个网盘按名字查找（逐页请求官方列表，条目多时要等一会儿）")
            onClicked: bar.searchAllRequested()
        }

        // 「退出」：没在搜时也要占位（visible 不参与布局），
        // 否则右边一出现按钮就会把整条输入框挤窄、文字跟着跳
        ToolButton {
            readonly property bool active: bar.backend !== null && bar.backend.searchActive
            text: qsTr("✕")
            visible: active
            opacity: active ? 1 : 0
            enabled: active
            ToolTip.visible: hovered
            ToolTip.text: qsTr("退出搜索，回到当前目录")
            onClicked: bar.exitRequested()
        }
    }
}
