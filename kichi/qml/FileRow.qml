import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 文件列表的一行（从 FilesPage.qml 抽出来：那里已到行数软上限）。
//
// 双击文件夹进入；播放 / 下载 / 重命名 / 删除都在右键菜单里。
// 条目字段来自 Rust 侧的 entry_map（与搜索结果同一套字段）。
ItemDelegate {
    id: row

    property var entry: null
    property var backend: null

    // 右键：相对本行的位置（交给页面换算到菜单坐标系）
    signal contextMenu(point position)

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

    // 整行双击：文件夹就进入。用 Control 自己的 onDoubleClicked
    // （Item 里没有这个信号，写 TapHandler 反而会和右键手势抢）。
    // 单击不再触发任何动作，避免手滑进目录。
    onDoubleClicked: row.open()

    function open() {
        if (row.backend && row.entry && row.entry.kind === "folder")
            row.backend.openDir(row.entry.id)
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
        spacing: 10

        Label {
            text: row.iconFor(row.entry)
            font.pixelSize: 18
        }

        Label {
            text: row.entry ? row.entry.name : ""
            elide: Text.ElideRight
            Layout.fillWidth: true
        }

        Label {
            text: qsTr("违规")
            visible: row.entry ? row.entry.auditFlag : false
            color: "#c0392b"
        }

        Label {
            text: row.entry ? row.entry.sizeText : ""
            color: palette.placeholderText
            horizontalAlignment: Text.AlignRight
            Layout.preferredWidth: 100
        }

        Label {
            readonly property string when: row.entry ? row.entry.modifiedText : ""
            text: when
            visible: when.length > 0
            color: palette.placeholderText
            horizontalAlignment: Text.AlignRight
            Layout.preferredWidth: 130
        }
    }

    HoverHandler {
        cursorShape: row.entry && row.entry.kind === "folder"
                     ? Qt.PointingHandCursor : Qt.ArrowCursor
    }
}
