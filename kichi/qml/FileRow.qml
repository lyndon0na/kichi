import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 文件列表的一行（从 FilesPage.qml 抽出来：那里已到行数软上限）。
//
// 点文件夹进入；右键交给页面的菜单；「▶」播放、「⬇」下载。
// 条目字段来自 Rust 侧的 entry_map（与搜索结果同一套字段）。
ItemDelegate {
    id: row

    property var entry: null
    property var backend: null

    // 右键：相对本行的位置（交给页面换算到菜单坐标系）
    signal contextMenu(point position)

    // 「▶」在页面坐标系里的按钮下沿中点（清晰度菜单从这里弹）
    signal playPressed(point anchor)

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

    // 整行点击：文件夹就进入。
    //
    // 用 ItemDelegate 自己的 onClicked（Control 的标准信号，Item 里没有
    // 这个信号，写 TapHandler 反而会和按钮抢手势）。
    // 右侧按钮各自消费点击，不会触发它。
    onClicked: row.open()

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

        // 只有视频/音频才显示播放入口
        ToolButton {
            text: qsTr("▶")
            visible: row.entry ? row.entry.playable === true : false
            enabled: row.backend !== null && !row.backend.busy
                     && row.backend.playMenu.length === 0
            ToolTip.visible: hovered
            ToolTip.text: qsTr("用 mpv 播放")

            onClicked: {
                row.playPressed(mapToItem(row, width / 2, height))
                row.backend.requestPlay(row.entry.id)
            }
        }

        // 下载到本地（文件直接下；文件夹递归下，保留层级）
        ToolButton {
            text: qsTr("⬇")
            enabled: row.backend !== null && !row.backend.downloadRunning
            ToolTip.visible: hovered
            ToolTip.text: row.entry && row.entry.kind === "folder"
                          ? qsTr("下载整个文件夹到本地")
                          : qsTr("下载到本地")

            onClicked: row.backend.downloadEntry(row.entry.id)
        }
    }

    HoverHandler {
        cursorShape: row.entry && row.entry.kind === "folder"
                     ? Qt.PointingHandCursor : Qt.ArrowCursor
    }
}
