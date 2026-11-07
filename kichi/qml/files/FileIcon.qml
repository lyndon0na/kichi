import QtQuick
import "../components"

// 文件类型图标：单色线稿，画法和侧栏导航同一套（NavIcon）。
//
// 文件页、搜索、分享、离线任务和传输任务都复用这里；「扩展名 → 图标」
// 只在此处维护。所有图标跟随主题描边色，不随字体尺寸变化。
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
        if (/\.(mp4|mkv|avi|mov|webm|ts|m2ts|m4v|flv|wmv|rmvb|rm|mpg|mpeg|mts|vob|3gp|ogv)$/.test(name))
            return "video"
        if (/\.(mp3|flac|wav|m4a|ogg|opus|aac|ape|wma|aiff|alac|mka|m4b)$/.test(name))
            return "music"
        if (/\.(jpg|jpeg|png|gif|webp|bmp|heic|heif|avif|svg|tif|tiff|ico|jxl|raw)$/.test(name))
            return "photo"
        if (/\.pdf$/.test(name))
            return "pdf"
        if (/\.(doc|docx|odt|rtf|txt|md)$/.test(name))
            return "document"
        if (/\.(xls|xlsx|csv|tsv|ods)$/.test(name))
            return "spreadsheet"
        if (/\.(ppt|pptx|odp|key)$/.test(name))
            return "presentation"
        if (/\.(zip|rar|7z|tar|gz|bz2|xz|zst|tgz|cab)$/.test(name))
            return "archive"
        if (/\.(rs|py|js|tsx|jsx|c|h|cpp|hpp|java|go|kt|sh)$/.test(name)
                || /\.(json|yaml|yml|toml|xml|html|css|sql)$/.test(name))
            return "code"
        if (/\.(srt|ass|ssa|vtt|sub|smi|sup|pgs|mks)$/.test(name))
            return "subtitle"
        if (/\.(exe|msi|apk|deb|rpm|appimage|pkg)$/.test(name))
            return "application"
        if (/\.torrent$/.test(name))
            return "torrent"
        return "file"
    }

    NavIcon {
        anchors.fill: parent
        name: icon.iconName
        color: icon.color
    }
}
