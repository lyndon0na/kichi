import QtQuick

// 文件类型图标：单色线稿，画法和侧栏导航同一套（NavIcon）。
//
// FileRow / SearchResults / TransferMainRow 都复用这里；「扩展名 → 图标」只在
// 此处维护。所有图标跟随主题描边色，不随字体尺寸变化。
Item {
    id: icon

    /// API 类型："folder" 是文件夹；"file"、未知或空时继续按扩展名细分
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
