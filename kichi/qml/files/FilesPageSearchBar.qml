import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "../components"

// 文件页的搜索框（挂在 main.qml 标题行中间；这里只管「搜索」这一件事）。
//
// 输入行为：
// - 输入阶段和没有索引时一致：只筛当前目录已加载条目（0 请求），清空 = 回当前目录；
// - 回车 / 放大镜 = 全盘搜索；索引关闭时先让用户确认开启，索引未就绪时
//   一次扫描同时出结果和建库，就绪后直接读本地库（0 请求）。
Item {
    id: bar

    property var backend: null

    implicitHeight: searchField.implicitHeight
    implicitWidth: 330

    // 输入防抖：连续输入只发最后一次查询
    Timer {
        id: debounce

        interval: 120
        repeat: false
        onTriggered: bar.applyLocalFilter()
    }

    // 输入防抖：清空就回到目录视图
    function applyLocalFilter() {
        if (!bar.backend)
            return
        if (searchField.text.trim().length === 0)
            bar.backend.exitSearch()
        else
            bar.backend.searchLocal(searchField.text)
    }

    // 全盘搜索入口：索引关闭时先确认，避免用户不知道这次会写本地库。
    function startFullScan() {
        if (!bar.backend || searchField.text.trim().length === 0)
            return
        if (bar.backend.localIndexEnabled === false) {
            indexPrompt.open()
            return
        }
        bar.runFullScan()
    }

    function runFullScan() {
        if (!bar.backend)
            return
        bar.backend.searchLocal(searchField.text)
        bar.backend.searchAll()
    }

    function clearAndExit() {
        searchField.text = ""
        if (bar.backend)
            bar.backend.exitSearch()
    }

    // 筛选变化：复用同一条 120ms 防抖再跑当前关键词。
    // 索引就绪时是本地索引重搜（0 请求）；未就绪时网络结果按已有命中重筛，
    // 不会在这里触发新的全盘扫描（ADR 0004）。
    function rerunAfterFilterChange() {
        if (!bar.backend || searchField.text.trim().length === 0)
            return
        debounce.restart()
    }

    KTextField {
        id: searchField

        objectName: "filesSearchField"
        anchors.fill: parent
        placeholderText: qsTr("搜索当前文件夹，回车全盘搜索")
        selectByMouse: true
        // 右边给内嵌的清空 / 搜索按钮让位，文字不钻到按钮底下
        rightPadding: buttonRow.width + 8

        onTextEdited: {
            if (searchField.text.trim().length === 0) {
                debounce.stop()
                bar.applyLocalFilter()
            } else {
                debounce.restart()
            }
        }
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
            onClicked: bar.clearAndExit()
        }

        // 全盘搜索：内嵌在输入框里，唯一会发起全盘搜索的按钮
        KIconButton {
            id: searchButton

            objectName: "filesSearchButton"
            iconName: "search"
            iconSize: 16
            iconOpacity: 1
            iconColor: palette.placeholderText
            enabled: searchField.text.trim().length > 0
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

    // 索引关闭时第一次全盘搜索：说明这次会同时建库，确认后由后端打开开关。
    KDialog {
        id: indexPrompt

        objectName: "searchIndexPrompt"
        parent: Overlay.overlay
        anchors.centerIn: Overlay.overlay
        title: qsTr("开启本地索引并全盘搜索")
        modal: true
        width: 440 + leftPadding + rightPadding

        footer: DialogFooter {
            outerMargin: indexPrompt.padding
            okText: qsTr("开启索引并全盘搜索")
            onOkClicked: {
                indexPrompt.close()
                bar.runFullScan()
            }
            onCancelClicked: indexPrompt.close()
        }

        Label {
            width: 440
            wrapMode: Text.Wrap
            text: qsTr("全盘搜索会同时建立本地索引，首次需要遍历整个网盘；之后的全盘搜索直接读本地库，不再请求网盘。")
        }
    }
}
