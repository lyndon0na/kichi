import QtQuick

// 文件类型图标：单色线稿，画法和侧栏导航同一套（NavIcon）。
//
// 为什么单独成一个组件：以前 FileRow / SearchResults / TransferRow 各自抄了
// 一份 emoji 表（📁 🎬 🎵 🖼️ 📄），三处会慢慢走偏；而且 emoji 是彩色字形，
// 尺寸随字体变、颜色改不了——列表里一排大黄文件夹、大小列对不齐都是它带来的。
// 现在「扩展名 → 图标」只在这里写一次。
Item {
    id: icon

    /// 条目类型："folder" 是文件夹，其余（file / 空）按扩展名判断
    property string kind: ""
    /// 文件名（只看扩展名）；传输记录只有路径，传 basename 也行
    property string fileName: ""
    /// 描边颜色，由调用方给（正文色、或降低不透明度表示次要）
    property color color: "transparent"

    implicitWidth: 18
    implicitHeight: 18

    readonly property string iconName: {
        if (icon.kind === "folder")
            return "folder"
        const name = String(icon.fileName || "").toLowerCase()
        if (/\.(mp4|mkv|avi|mov|webm|ts|m2ts|flv|wmv|rmvb)$/.test(name))
            return "video"
        if (/\.(mp3|flac|wav|m4a|ogg|opus|aac|ape)$/.test(name))
            return "music"
        if (/\.(jpg|jpeg|png|gif|webp|bmp|heic|avif)$/.test(name))
            return "photo"
        return "file"
    }

    NavIcon {
        anchors.fill: parent
        name: icon.iconName
        color: icon.color
    }
}
