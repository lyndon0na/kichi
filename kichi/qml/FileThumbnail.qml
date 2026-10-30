import QtQuick
import QtQuick.Shapes

// 文件列表的首列：固定 36×24，不随内容改变列位置。
//
// 图片 / 视频加载成功时铺满裁切；文件夹、不支持预览、远程失败、
// 本地图片解码失败都退回原来的 FileIcon。视频成功缩略图右下角有小角标。
Item {
    id: control

    property var backend: null
    property var entry: null
    property int iconPixelSize: 18
    property int radius: 3
    /// 视频角标尺寸：列表用小角标，网格可以放大
    property int videoBadgeSize: 12

    /// Rust 回传的是 file:// URI，测试里也可以注入其它本地 source。
    property string localUri: ""
    /// 当前 id 已经请求过：组件在同一次委托生命周期里避免重复发请求。
    property string requestedId: ""

    readonly property bool imageReady: localUri.length > 0 && preview.status === Image.Ready
    readonly property bool showVideoBadge: imageReady
                                            && entry !== null
                                            && entry.video === true

    implicitWidth: 36
    implicitHeight: 24

    readonly property KTheme theme: KTheme { parent: control }

    function clearAndFallback() {
        localUri = ""
    }

    function refresh() {
        localUri = ""
        if (!entry || entry.hasThumbnail !== true)
            return
        if (!backend)
            return

        if (typeof backend.thumbnailPath === "function") {
            var cached = backend.thumbnailPath(String(entry.id))
            if (cached && cached.length > 0) {
                localUri = cached
                return
            }
        }
        var entryId = String(entry.id)
        if (typeof backend.requestThumbnail === "function" && requestedId !== entryId) {
            requestedId = entryId
            backend.requestThumbnail(entryId)
        }
    }

    onEntryChanged: refresh()
    onBackendChanged: refresh()
    Component.onCompleted: refresh()

    Connections {
        target: control.backend
        // 测试和只读占位后端不一定声明缩略图信号；真实 Kichi 后端会声明。
        ignoreUnknownSignals: true

        function onThumbnailReady(id, path) {
            if (control.entry && id === String(control.entry.id))
                control.localUri = path || ""
        }
    }

    Rectangle {
        id: frame

        anchors.fill: parent
        radius: control.radius
        color: control.localUri.length > 0 ? control.theme.surfaceRaised
                                           : control.theme.surfaceClear
        border.width: control.imageReady ? 1 : 0
        border.color: control.theme.hairline
        clip: true

        Image {
            id: preview
            objectName: "thumbnailImage"

            anchors.fill: parent
            source: control.localUri
            asynchronous: true
            cache: true
            mipmap: true
            fillMode: Image.PreserveAspectCrop
            sourceSize.width: Math.max(1, Math.round(control.width * 2))
            sourceSize.height: Math.max(1, Math.round(control.height * 2))
            visible: control.imageReady

            onStatusChanged: {
                if (status === Image.Error)
                    control.clearAndFallback()
            }
        }
    }

    FileIcon {
        id: fallback
        objectName: "thumbnailFallbackIcon"

        width: Math.min(control.iconPixelSize, control.width)
        height: width
        anchors.centerIn: parent
        visible: !control.imageReady
        kind: control.entry ? control.entry.kind : ""
        fileName: control.entry ? control.entry.name : ""
        color: control.theme.textPrimary
        opacity: control.entry && control.entry.kind === "folder" ? 1.0 : 0.6
    }

    Rectangle {
        objectName: "videoBadge"

        visible: control.showVideoBadge
        width: control.videoBadgeSize
        height: control.videoBadgeSize
        radius: Math.max(2, Math.round(control.videoBadgeSize / 6))
        anchors.right: parent.right
        anchors.rightMargin: 2
        anchors.bottom: parent.bottom
        anchors.bottomMargin: 2
        color: control.theme.mediaBadge

        Item {
            width: 6
            height: 8
            anchors.centerIn: parent
            scale: control.videoBadgeSize / 12

            Shape {
                anchors.fill: parent

                ShapePath {
                    fillColor: control.theme.onMediaBadge
                    strokeColor: "transparent"

                    PathSvg {
                        path: "M0 0 L6 4 L0 8 Z"
                    }
                }
            }
        }
    }
}
