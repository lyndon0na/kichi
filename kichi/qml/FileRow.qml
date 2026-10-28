import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 文件列表的一行（从 FilesPage.qml 抽出来：那里已到行数软上限）。
//
// 单击选中（highlighted 由页面按 id 绑定）；双击：文件夹进入、
// 媒体文件播放、其余文件下载。重命名 / 删除等仍在右键菜单里。
// 条目字段来自 Rust 侧的 entry_map（与搜索结果同一套字段）。
ItemDelegate {
    id: row

    property var entry: null
    property var backend: null
    // 列表视图本身。⚠ 只能取在委托根上：`ListView.view` 附加属性只在委托根
    // 有效，写在下层 MouseArea 里永远是 null（子项拿不到），
    // 表现为「点了行，快捷键还是没焦点」。
    readonly property var listView: ListView.view
    /// 在列表里的下标（Shift 区间选择用）
    property int rowIndex: -1
    // 列宽由页面统一给出，与表头（FileListHeader）同一组数值
    property int iconColumnWidth: 30
    property int sizeColumnWidth: 126
    property int modifiedColumnWidth: 165
    property int columnSpacing: 10
    // 行高 / 字号 / 图标大小同样由页面给出（列表密度集中在 FilesPage.qml）
    property int rowHeight: 52
    property int textPixelSize: 16
    property int iconPixelSize: 24
    // 水平内边距固定：表头按同一个数值摆位。不依赖控件风格的默认 padding，
    // 否则换风格（这里实测是 8 而不是 Qt 默认的 12）表头就和行错开。
    leftPadding: 8
    rightPadding: 8

    // 行高显式写死：ItemDelegate 自己算的是「文字高度 + 上下 padding」，
    // 跟着字号漂，和表头对不齐。这里由页面给一个确定值。
    // 内容仍是 RowLayout 填满整行，Label 默认 Text.AlignVCenter，文字自动居中。
    implicitHeight: row.rowHeight

    // 右键：相对本行的位置（交给页面换算到菜单坐标系）
    signal contextMenu(point position)
    // 双击媒体文件：把条目 id 和行内锚点交给页面（清晰度菜单从这里弹出）
    signal playRequested(string id, point position)
    // 单击：把修饰键（Ctrl / Shift）与行下标交给页面做多选
    signal selectRequested(int modifiers, int index)
    // 双击文件夹：交给页面先记滚动位置再进目录
    signal folderOpenRequested(string id)

    // 图标按扩展名猜（📁 🎬 🎵 🖼️ 📄）。
    // 不用 `import "...js"`：外部脚本不进 qrc（实测打包后 404），
    // 图标逻辑很小，直接内联。
    function iconFor(entry) {
        if (!entry)
            return "📄"
        if (entry.kind === "folder")
            return "📁"
        const name = String(entry.name || "").toLowerCase()
        if (/\.(mp4|mkv|avi|mov|webm|ts|m2ts|flv|wmv|rmvb)$/.test(name))
            return "🎬"
        if (/\.(mp3|flac|wav|m4a|ogg|opus|aac|ape)$/.test(name))
            return "🎵"
        if (/\.(jpg|jpeg|png|gif|webp|bmp|heic|avif)$/.test(name))
            return "🖼️"
        return "📄"
    }

    icon.name: ""
    icon.source: ""
    icon.width: 0
    icon.height: 0
    icon.color: "transparent"

    // 整行手势。
    //
    // 为什么不用 ItemDelegate 自带的 onClicked / onDoubleClicked：它们拿不到
    // 键盘修饰键，而 Ctrl / Shift 多选必须读 `mouse.modifiers`。TapHandler 的
    // eventPoint 也不带修饰键，所以整行铺一个 MouseArea。
    //
    // 注意：MouseArea 会吃掉按下的鼠标事件，Flickable 不再能用鼠标拖动滚动；
    // 滚轮不受影响（原本 ItemDelegate 也会吃掉按下，行为没有退化）。
    MouseArea {
        id: mouseArea

        anchors.fill: parent
        // 盖在 contentItem 之上，保证整行都可点
        z: 1
        acceptedButtons: Qt.LeftButton | Qt.RightButton
        cursorShape: row.entry && row.entry.kind === "folder"
                     ? Qt.PointingHandCursor : Qt.ArrowCursor

        onClicked: function (mouse) {
            if (row.listView)
                row.listView.forceActiveFocus()
            if (mouse.button === Qt.RightButton) {
                if (row.entry)
                    row.contextMenu(Qt.point(mouse.x, mouse.y))
                return
            }
            row.selectRequested(mouse.modifiers, row.rowIndex)
        }

        onDoubleClicked: function (mouse) {
            if (mouse.button === Qt.LeftButton)
                row.activate(Qt.point(mouse.x, mouse.y))
        }
    }

    // 双击的实际动作；单击选中由页面处理，不在这里做。
    function activate(point) {
        if (!row.backend || !row.entry)
            return
        if (row.entry.kind === "folder") {
            row.folderOpenRequested(row.entry.id)
            return
        }
        if (row.entry.playable === true) {
            row.playRequested(row.entry.id, point)
            return
        }
        row.backend.downloadEntry(row.entry.id)
    }

    // 选中态用主题高亮色；未选中时透明，让列表背景透出来。
    // 不用 ItemDelegate 默认 background：Basic 风格的 highlighted 只画一圈边框，
    // 在长列表里几乎看不出来选中了哪行。
    background: Rectangle {
        radius: 4
        color: row.highlighted ? palette.highlight
             : mouseArea.pressed ? palette.mid
             : "transparent"
    }

    contentItem: RowLayout {
        spacing: row.columnSpacing

        // 固定宽度：图标宽度随 emoji 字形变，表头的「名称」就对不上文件名了
        Label {
            text: row.iconFor(row.entry)
            font.pixelSize: row.iconPixelSize
            Layout.preferredWidth: row.iconColumnWidth
            horizontalAlignment: Text.AlignHCenter
            verticalAlignment: Text.AlignVCenter
            color: row.highlighted ? palette.highlightedText : palette.text
        }

        Label {
            text: row.entry ? row.entry.name : ""
            font.pixelSize: row.textPixelSize
            verticalAlignment: Text.AlignVCenter
            elide: Text.ElideRight
            Layout.fillWidth: true
            color: row.highlighted ? palette.highlightedText : palette.text
        }

        Label {
            text: qsTr("违规")
            visible: row.entry ? row.entry.auditFlag : false
            font.pixelSize: row.textPixelSize
            verticalAlignment: Text.AlignVCenter
            color: row.highlighted ? palette.highlightedText : "#c0392b"
        }

        Label {
            text: row.entry ? row.entry.sizeText : ""
            font.pixelSize: row.textPixelSize
            verticalAlignment: Text.AlignVCenter
            color: row.highlighted ? palette.highlightedText : palette.placeholderText
            opacity: row.highlighted ? 0.8 : 1
            horizontalAlignment: Text.AlignRight
            Layout.preferredWidth: row.sizeColumnWidth
        }

        // 没有修改时间的条目也占住整列宽度：否则这一列会塌掉，
        // 大小列跟着右移，表头就对不齐了
        Label {
            text: row.entry ? row.entry.modifiedText : ""
            font.pixelSize: row.textPixelSize
            verticalAlignment: Text.AlignVCenter
            color: row.highlighted ? palette.highlightedText : palette.placeholderText
            opacity: row.highlighted ? 0.8 : 1
            horizontalAlignment: Text.AlignRight
            Layout.preferredWidth: row.modifiedColumnWidth
        }
    }
}
