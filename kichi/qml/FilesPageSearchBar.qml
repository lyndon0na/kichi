import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 文件页的搜索框（挂在 main.qml 标题行中间；这里只管「搜索」这一件事）。
//
// 两段式（docs/adr/0004）：
// - 输入 = 只筛当前目录已加载的条目（0 请求），清空 = 回当前目录；
// - 回车 / 点输入框里内嵌的放大镜 = 全盘搜索（唯一会发请求的入口）。
Item {
    id: bar

    property var backend: null

    // 扫描进行中：按钮置灰；停止入口在结果头里（SearchResults.qml）
    readonly property bool scanning: backend !== null
                                      && backend.searchActive
                                      && !backend.searchSettled

    implicitHeight: searchField.implicitHeight
    implicitWidth: 330

    // 输入即筛当前目录：清空就回到目录视图
    function applyLocalFilter() {
        if (!bar.backend)
            return
        if (searchField.text.trim().length === 0)
            bar.backend.exitSearch()
        else
            bar.backend.searchLocal(searchField.text)
    }

    // 全盘搜索：唯一会发请求的入口
    function startFullScan() {
        if (!bar.backend || searchField.text.trim().length === 0)
            return
        bar.backend.searchAll()
    }

    function clearAndExit() {
        searchField.text = ""
        if (bar.backend)
            bar.backend.exitSearch()
    }

    KTextField {
        id: searchField

        objectName: "filesSearchField"
        anchors.fill: parent
        placeholderText: qsTr("搜索当前文件夹，回车全盘搜索")
        selectByMouse: true
        // 右边给内嵌的清空 / 搜索按钮让位，文字不钻到按钮底下
        rightPadding: buttonRow.width + 8

        onTextEdited: bar.applyLocalFilter()
        onAccepted: bar.startFullScan()

        // Esc：有字先清空回当前目录，已空则退出搜索视图（与改动前一致）。
        // TextField 自己会「消费」Esc，必须先在 ShortcutOverride 里接住。
        Keys.onShortcutOverride: function (event) {
            if (event.key === Qt.Key_Escape)
                event.accepted = true
        }
        Keys.onEscapePressed: {
            if (searchField.text.length > 0)
                bar.clearAndExit()
            else if (bar.backend && bar.backend.searchActive)
                bar.backend.exitSearch()
        }
    }

    // 内嵌按钮叠在输入框右侧。注意不能塞进 TextField 里当子项：
    // TextField 的默认属性是 text（字符串），塞子项会直接加载失败。
    Row {
        id: buttonRow

        anchors.right: parent.right
        anchors.rightMargin: 3
        anchors.verticalCenter: parent.verticalCenter
        spacing: 1

        // 有字才出现：清空关键词并回到当前文件夹
        KIconButton {
            id: clearButton

            iconName: "x"
            iconSize: 12
            iconOpacity: 0.7
            iconColor: palette.placeholderText
            visible: searchField.text.length > 0
            ToolTip.visible: hovered
            ToolTip.text: qsTr("清空关键词，回到当前文件夹")
            onClicked: bar.clearAndExit()
        }

        // 全盘搜索：内嵌在输入框里，唯一会发请求的按钮
        KIconButton {
            id: searchButton

            iconName: "search"
            iconSize: 16
            iconOpacity: 1
            iconColor: palette.placeholderText
            enabled: searchField.text.trim().length > 0 && !bar.scanning
            ToolTip.visible: hovered
            ToolTip.text: qsTr("遍历整个网盘按名字查找（逐页请求官方列表，条目多时要等一会儿）")
            onClicked: bar.startFullScan()
        }
    }

    // 搜索在别处结束（点了结果里的文件夹、离开文件页）时清掉关键词：
    // 否则框里留着旧字，一按回车又搜一遍
    Connections {
        target: bar.backend

        function onChanged() {
            if (!bar.backend)
                return
            if (!bar.backend.searchActive && searchField.text.length > 0)
                searchField.text = ""
        }
    }
}
