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
    // 列宽由页面统一给出，与表头（FileListHeader）同一组数值
    property int iconColumnWidth: 24
    property int sizeColumnWidth: 100
    property int modifiedColumnWidth: 130
    property int columnSpacing: 10
    // 水平内边距固定：表头按同一个数值摆位。不依赖控件风格的默认 padding，
    // 否则换风格（这里实测是 8 而不是 Qt 默认的 12）表头就和行错开。
    leftPadding: 8
    rightPadding: 8

    // 右键：相对本行的位置（交给页面换算到菜单坐标系）
    signal contextMenu(point position)
    // 双击媒体文件：把条目 id 和行内锚点交给页面（清晰度菜单从这里弹出）
    signal playRequested(string id, point position)

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

    // 整行双击：文件夹进入；媒体文件播放；其余文件下载到本地。
    // 用 Control 自己的 onDoubleClicked（Item 里没有这个信号，
    // 写 TapHandler 反而会和右键手势抢）。
    // 单击只选中、不触发动作（单击进目录太容易手滑）。
    onDoubleClicked: row.activate()

    // 双击的实际动作；单击选中由页面绑定，不在这里做。
    function activate() {
        if (!row.backend || !row.entry)
            return
        if (row.entry.kind === "folder") {
            row.backend.openDir(row.entry.id)
            return
        }
        if (row.entry.playable === true) {
            // 锚点用行底中点：和右键菜单一样换算到页面坐标系
            row.playRequested(row.entry.id, Qt.point(row.width / 2, row.height))
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
             : row.down ? palette.mid
             : "transparent"
    }

    TapHandler {
        acceptedButtons: Qt.RightButton
        onTapped: function (eventPoint) {
            if (!row.entry)
                return
            // 只上报行内坐标。菜单的坐标系由页面负责换算，
            // 这里先转成窗口坐标会被 popup 再偏移一次。
            row.contextMenu(eventPoint.position)
        }
    }

    contentItem: RowLayout {
        spacing: row.columnSpacing

        // 固定宽度：图标宽度随 emoji 字形变，表头的「名称」就对不上文件名了
        Label {
            text: row.iconFor(row.entry)
            font.pixelSize: 18
            Layout.preferredWidth: row.iconColumnWidth
            horizontalAlignment: Text.AlignHCenter
            color: row.highlighted ? palette.highlightedText : palette.text
        }

        Label {
            text: row.entry ? row.entry.name : ""
            elide: Text.ElideRight
            Layout.fillWidth: true
            color: row.highlighted ? palette.highlightedText : palette.text
        }

        Label {
            text: qsTr("违规")
            visible: row.entry ? row.entry.auditFlag : false
            color: row.highlighted ? palette.highlightedText : "#c0392b"
        }

        Label {
            text: row.entry ? row.entry.sizeText : ""
            color: row.highlighted ? palette.highlightedText : palette.placeholderText
            opacity: row.highlighted ? 0.8 : 1
            horizontalAlignment: Text.AlignRight
            Layout.preferredWidth: row.sizeColumnWidth
        }

        // 没有修改时间的条目也占住整列宽度：否则这一列会塌掉，
        // 大小列跟着右移，表头就对不齐了
        Label {
            text: row.entry ? row.entry.modifiedText : ""
            color: row.highlighted ? palette.highlightedText : palette.placeholderText
            opacity: row.highlighted ? 0.8 : 1
            horizontalAlignment: Text.AlignRight
            Layout.preferredWidth: row.modifiedColumnWidth
        }
    }

    HoverHandler {
        cursorShape: row.entry && row.entry.kind === "folder"
                     ? Qt.PointingHandCursor : Qt.ArrowCursor
    }
}
