import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 文件页（S3）：面包屑导航 + 目录列表。
// 只调 backend 的 Q_INVOKABLE；配额只在侧栏账号卡里显示（SideBar.qml），这里不重复。
// 播放（S5）：右键菜单点「播放」→ backend 取档位 → playMenu 非空时弹清晰度菜单。
// 搜索（S9）：搜索框在标题行（main.qml 里的 FilesPageSearchBar），输入只筛当前目录，
// 全盘搜索由回车 / 内嵌按钮触发；结果在 SearchResults.qml 里，和目录列表共用同一块列表区。
Item {
    id: page

    property var backend: null
    // 右键菜单里点「播放」处的位置（页面坐标系，清晰度菜单从这里弹出）
    property point playAnchor: Qt.point(0, 0)
    // 当前目录列表里选中的条目 id（空串 = 没有选中）
    property string selectedId: ""

    // 搜索视图开着：列表区换成结果
    readonly property bool searching: page.backend !== null && page.backend.searchActive

    // 列表列宽：表头（FileListHeader）与每一行（FileRow）共用同一组数值。
    // 只在这里改，两边一起跟着变，不会出现「表头 100px、行里 90px」的错位。
    readonly property int iconColumnWidth: 24
    readonly property int sizeColumnWidth: 100
    readonly property int modifiedColumnWidth: 130
    // 行的水平内边距：FileRow 显式用它，表头也用它，不跟控件风格的默认值走
    readonly property int rowPadding: 8
    readonly property int columnSpacing: 10

    function crumbCount() {
        return page.backend ? page.backend.breadcrumb.length : 0
    }

    function isCurrentCrumb(index) {
        return index === page.crumbCount() - 1
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
    //
    // 搜索框本身在标题行（FilesPageSearchBar.qml）：输入只筛本目录，
    // 回车 / 内嵌按钮才全盘搜。本页只负责显示结果和从结果跳转。

    // 标题行的「＋ 新建文件夹」调这里：弹窗和菜单都在本页（FilesPageMenu）
    function beginNewFolder() {
        menu.openNewFolder()
    }

    // 搜索结果里的播放 / 下载也在右键菜单里，锚点用右键位置
    function openResultContext(entry, point) {
        var menuPoint = searchResults.mapToItem(menu, point.x, point.y)
        menu.openContext(entry, menuPoint)
    }

    // 切到别的页时把搜索收掉：回来看到的是当前目录，而不是忘在一边的旧结果
    // （搜索框里的字由 FilesPageSearchBar 自己跟着 searchActive 清）
    onVisibleChanged: {
        if (!page.visible && page.searching && page.backend)
            page.backend.exitSearch()
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 10

        // —— 导航条：上一级（图标）+ 面包屑 ——
        // 正在加载的那一级名字后面跟着转圈，其余位置不放指示器。
        RowLayout {
            Layout.fillWidth: true
            spacing: 6

            ToolButton {
                text: qsTr("↑")
                enabled: page.backend !== null && !page.backend.busy && page.crumbCount() > 1
                ToolTip.visible: hovered
                ToolTip.text: qsTr("上一级（回到上级目录）")
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

                        // 加载中转圈：只跟在当前目录名后面
                        BusyIndicator {
                            visible: page.isCurrentCrumb(index)
                                     && page.backend !== null && page.backend.busy
                            running: visible
                            Layout.preferredWidth: 18
                            Layout.preferredHeight: 18
                            Layout.alignment: Qt.AlignVCenter
                        }
                    }
                }
            }
        }

        // —— 列表（当前目录 / 搜索结果共用这块区域）——
        // 不用 Frame：它的边框画在外层，会被盖住的上下两条边吃掉，
        // 只剩左右两段淡线，看起来像对不齐的框。列表直接铺在背景上。
        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true

            ColumnLayout {
                anchors.fill: parent
                spacing: 4

                // 表头只属于当前目录视图：搜索结果列不同（多类型列、所在文件夹），
                // 不硬套同一套列宽。排序箭头读的是 backend 的排序状态。
                FileListHeader {
                    visible: !page.searching
                    Layout.fillWidth: true
                    backend: page.backend
                    iconColumnWidth: page.iconColumnWidth
                    sizeColumnWidth: page.sizeColumnWidth
                    modifiedColumnWidth: page.modifiedColumnWidth
                    contentPadding: page.rowPadding
                    columnSpacing: page.columnSpacing
                }

                ListView {
                    id: fileList

                    Layout.fillWidth: true
                    Layout.fillHeight: true
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
                        iconColumnWidth: page.iconColumnWidth
                        sizeColumnWidth: page.sizeColumnWidth
                        modifiedColumnWidth: page.modifiedColumnWidth
                        columnSpacing: page.columnSpacing
                        // 单击选中；id 绑定而不是 index，刷新 / 排序后仍指向同一条目
                        highlighted: page.selectedId.length > 0
                                     && !!entry && page.selectedId === entry.id
                        onClicked: page.selectedId = entry.id
                        // 双击媒体文件：页面把它换算到页面坐标系再弹清晰度菜单
                        onPlayRequested: function (id, point) {
                            page.playAnchor = row.mapToItem(page, point.x, point.y)
                            if (page.backend)
                                page.backend.requestPlay(id)
                        }
                        // FileRow 给的是行内坐标；菜单 root 与页面同尺寸，
                        // 直接映射到菜单坐标系才能和 popup(x, y) 对齐。
                        onContextMenu: function (point) {
                            page.selectedId = entry.id
                            var menuPoint = row.mapToItem(menu, point.x, point.y)
                            menu.openContext(entry, menuPoint)
                        }
                    }
                }
            }

            // 居中对齐到列表区域：表头在时提示落在列表中间，不会压在表头上
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

                onContextMenu: function (entry, point) {
                    page.openResultContext(entry, point)
                }
                onPlayRequested: function (id, point) {
                    page.playAnchor = searchResults.mapToItem(page, point.x, point.y)
                    if (page.backend)
                        page.backend.requestPlay(id)
                }
            }
        }

        // —— 下载进度（有下载在跑时出现；点 × 取消）——
        DownloadBar {
            Layout.fillWidth: true
            backend: page.backend
        }
    }

    // —— 右键菜单与重命名 / 新建 / 删除弹窗（内容在 FilesPageMenu.qml）——
    FilesPageMenu {
        id: menu
        anchors.fill: parent
        backend: page.backend
        onPlayRequested: function (anchor) {
            page.playAnchor = anchor
        }
    }

    // —— 清晰度菜单（内容在 FilesPagePlayMenu.qml）——
    FilesPagePlayMenu {
        id: playMenu
        backend: page.backend
        anchorPoint: page.playAnchor
    }
}
