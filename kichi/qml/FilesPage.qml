import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 文件页（S3）：面包屑导航 + 目录列表。
// 只调 backend 的 Q_INVOKABLE；配额沿用顶栏的 quotaSummary，不重复显示。
// 播放（S5）：点行尾「▶」→ backend 取档位 → playMenu 非空时弹清晰度菜单。
// 搜索（S9）：工具条搜索框先在当前目录本地筛（0 请求），「全盘搜索」才遍历网盘；
// 搜索结果在 SearchResults.qml 里，和目录列表共用同一块列表框。
Item {
    id: page

    property var backend: null
    // 「▶」按钮在页面坐标系里的位置（菜单从这里弹出）
    property point playAnchor: Qt.point(0, 0)

    // 搜索视图开着：列表区换成结果
    readonly property bool searching: page.backend !== null && page.backend.searchActive

    function crumbCount() {
        return page.backend ? page.backend.breadcrumb.length : 0
    }

    function isCurrentCrumb(index) {
        return index === page.crumbCount() - 1
    }

    function sortLabel() {
        if (!page.backend)
            return ""
        var field = page.backend.sortField
        var name = field === "size" ? qsTr("大小")
                 : field === "modified" ? qsTr("时间")
                 : qsTr("名称")
        return name + (page.backend.sortAscending ? " ↑" : " ↓")
    }

    // 点一下换字段，再点当前字段则翻方向（与老习惯一致）
    function cycleSort() {
        if (!page.backend)
            return
        var order = ["name", "modified", "size"]
        var index = order.indexOf(page.backend.sortField)
        if (index < 0)
            index = 0
        if (index + 1 < order.length)
            page.backend.setSorting(order[index + 1], page.backend.sortAscending)
        else
            page.backend.setSorting(page.backend.sortField, !page.backend.sortAscending)
    }

    function iconFor(item) {
        if (item.kind === "folder")
            return "📁"
        const name = item.name.toLowerCase()
        if (/\.(mp4|mkv|avi|mov|webm|ts|m2ts|flv|wmv|rmvb)$/.test(name))
            return "🎬"
        if (/\.(mp3|flac|wav|m4a|ogg|opus|aac|ape)$/.test(name))
            return "🎵"
        if (/\.(jpg|jpeg|png|gif|webp|bmp|heic|avif)$/.test(name))
            return "🖼️"
        return "📄"
    }

    // —— 搜索入口 ——

    // 输入即筛当前目录（0 请求）；清空就退出搜索视图
    function submitSearch(text) {
        if (!page.backend)
            return
        if (String(text || "").trim().length === 0)
            page.backend.exitSearch()
        else
            page.backend.searchLocal(text)
    }

    // 只在「还没在搜、搜索框也没字」时跳到搜索框：
    // 正在搜时 `/` 要能正常打进关键词里
    function focusSearchIfIdle() {
        if (!page.backend || page.backend.authState !== "signedIn")
            return false
        if (page.searching || searchBar.text.length > 0)
            return false
        searchBar.field.forceActiveFocus()
        return true
    }

    // Esc：搜索框有字 → 清空并回到当前目录；已空 → 退出搜索视图
    function escapeSearch() {
        if (!page.backend)
            return
        if (searchBar.text.length > 0) {
            searchBar.text = ""
            page.backend.exitSearch()
        } else if (page.searching) {
            page.backend.exitSearch()
        }
    }

    // 搜索结果里的「▶」：把结果行里的坐标换算到页面坐标系，
    // 清晰度菜单才弹在对的位置
    function playFromResults(anchor) {
        page.playAnchor = searchResults.mapToItem(page, anchor.x, anchor.y)
    }

    // 搜索被别处结束（点了结果里的文件夹）时清掉关键词，
    // 否则搜索框留着旧字、一点「全盘搜索」又开搜
    function syncSearchField() {
        if (!page.backend)
            return
        if (!page.backend.searchActive && searchBar.text.length > 0)
            searchBar.text = ""
    }

    // 切到别的页时把搜索收掉：回来看到的是当前目录，而不是忘在一边的旧结果
    onVisibleChanged: {
        if (!page.visible && page.searching && page.backend)
            page.backend.exitSearch()
    }

    // —— 键盘：`/` 聚焦搜索框，Esc 逐级退出搜索 ——
    // 挂在最外层 Item 上：没有输入框抢焦点时，按键才会冒泡到这里。
    focus: false
    Keys.onPressed: function (event) {
        if (!page.backend)
            return
        if (event.key === Qt.Key_Slash) {
            if (page.focusSearchIfIdle())
                event.accepted = true
        } else if (event.key === Qt.Key_Escape) {
            if (page.searching || searchBar.text.length > 0) {
                page.escapeSearch()
                event.accepted = true
            }
        }
    }

    Connections {
        target: page.backend

        function onChanged() {
            if (page.backend)
                page.syncSearchField()
        }
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 10

        // —— 导航条 ——
        RowLayout {
            Layout.fillWidth: true
            spacing: 6

            ToolButton {
                text: qsTr("↑ 上一级")
                enabled: page.backend !== null && !page.backend.busy && page.crumbCount() > 1
                onClicked: page.backend.openParent()
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: 2

                Repeater {
                    model: page.backend ? page.backend.breadcrumb : []

                    delegate: RowLayout {
                        spacing: 2

                        Label {
                            text: "›"
                            visible: index > 0
                            color: palette.placeholderText
                        }

                        Label {
                            text: modelData
                            font.bold: page.isCurrentCrumb(index)
                            color: page.isCurrentCrumb(index) ? palette.text : palette.link
                            elide: Text.ElideMiddle
                            Layout.maximumWidth: 220

                            MouseArea {
                                anchors.fill: parent
                                enabled: !page.isCurrentCrumb(index)
                                cursorShape: enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
                                onClicked: page.backend.openCrumb(index)
                            }
                        }
                    }
                }
            }

            BusyIndicator {
                running: page.backend !== null && page.backend.busy
                visible: running
                Layout.preferredWidth: 24
                Layout.preferredHeight: 24
            }

            ToolButton {
                text: page.sortLabel()
                enabled: page.backend !== null
                ToolTip.visible: hovered
                ToolTip.text: qsTr("点按切换排序字段（名称 → 时间 → 大小），循环到末尾再点翻转方向")
                onClicked: page.cycleSort()
            }

            ToolButton {
                text: qsTr("刷新")
                enabled: page.backend !== null && !page.backend.busy
                visible: !page.searching
                onClicked: page.backend.refreshFiles()
            }

            ToolButton {
                text: qsTr("＋ 新建文件夹")
                enabled: page.backend !== null && !page.backend.busy
                         && !page.backend.fileActionRunning
                visible: !page.searching
                onClicked: menu.openNewFolder()
            }
        }

        // —— 搜索条（工具条下方一行；细节在 FilesPageSearchBar.qml）——
        FilesPageSearchBar {
            id: searchBar
            Layout.fillWidth: true
            backend: page.backend
            onQueryEdited: function (text) {
                page.submitSearch(text)
            }
            onExitRequested: page.backend.exitSearch()
            onSearchAllRequested: page.backend.searchAll()
        }

        // —— 列表（当前目录 / 搜索结果共用这块区域）——
        Frame {
            Layout.fillWidth: true
            Layout.fillHeight: true
            padding: 0

            ListView {
                id: fileList

                anchors.fill: parent
                clip: true
                // 搜索视图显示结果；否则显示当前目录
                visible: !page.searching
                model: page.backend ? page.backend.files : []
                boundsBehavior: Flickable.StopAtBounds
                ScrollBar.vertical: ScrollBar {
                    policy: ScrollBar.AsNeeded
                }

                delegate: FileRow {
                    id: row
                    width: ListView.view.width
                    entry: modelData
                    backend: page.backend
                    // 右键菜单坐标相对整个页面：先换算再交给菜单
                    onContextMenu: function (point) {
                        menu.openContext(entry, point)
                    }
                    onPlayPressed: function (anchor) {
                        page.playAnchor = row.mapToItem(page, anchor.x, anchor.y)
                    }
                }
            }

            Label {
                anchors.centerIn: parent
                text: qsTr("这个文件夹是空的")
                color: palette.placeholderText
                visible: !page.searching && page.backend !== null && !page.backend.busy
                         && page.backend.files.length === 0
            }

            // 搜索结果（全盘搜索时才出现，盖住同一块区域）
            SearchResults {
                id: searchResults
                anchors.fill: parent
                // 必须真隐藏：它内部的 ListView（Flickable）会吃掉鼠标按压。
                // 只把 backend 置空、不设 visible 时，它透明地盖在目录列表上，
                // 状态行以下的区域全被它截走——表现就是「只有第一行能点」
                visible: page.searching
                backend: page.searching ? page.backend : null

                onPlayRequested: function (anchor) {
                    page.playFromResults(anchor)
                }
            }
        }

        // —— 下载进度（有下载在跑时出现；点 × 取消）——
        DownloadBar {
            Layout.fillWidth: true
            backend: page.backend
        }

        Label {
            text: qsTr("点视频/音频右侧的「▶」用 mpv 播放；点「⬇」下载到本地（文件夹会连同子目录一起下）。右键条目可重命名或移到回收站。下载目录在设置页里改。")
            color: palette.placeholderText
            font.pixelSize: 12
            wrapMode: Text.Wrap
            Layout.fillWidth: true
        }
    }

    // —— 右键菜单与重命名 / 新建 / 删除弹窗（内容在 FilesPageMenu.qml）——
    FilesPageMenu {
        id: menu
        anchors.fill: parent
        backend: page.backend
    }

    // —— 清晰度菜单（内容在 FilesPagePlayMenu.qml）——
    FilesPagePlayMenu {
        id: playMenu
        backend: page.backend
        anchorPoint: page.playAnchor
    }
}
