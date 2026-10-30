import QtQuick
import QtQuick.Shapes
import QtQuick.Effects

// 文件列表的首列：固定 36×24，不随内容改变列位置。
//
// 图片 / 视频加载成功时铺满裁切；文件夹、不支持预览、远程失败、
// 本地图片解码失败都退回原来的 FileIcon。可播放视频无论有没有缩略图都保留
// 右下角描边播放角标；真正开播后改为强调色内环，加载时由 VideoLoadingOverlay 覆盖。
Item {
    id: control

    property var backend: null
    property var entry: null
    property int iconPixelSize: 18
    property int radius: 3
    /// 视频角标尺寸：列表用小角标，网格可以放大
    property int videoBadgeSize: 16
    property int videoBadgeMargin: 2
    /// 播放加载态优先于可播放角标：转圈出现时只保留一个视觉焦点。
    property bool loading: false
    /// 播放器已经真正开播、尚未退出
    property bool playing: false

    /// Rust 回传的是 file:// URI，测试里也可以注入其它本地 source。
    property string localUri: ""
    /// 当前 id 已经请求过：组件在同一次委托生命周期里避免重复发请求。
    property string requestedId: ""

    readonly property bool imageReady: localUri.length > 0 && preview.status === Image.Ready
    readonly property bool showVideoBadge: entry !== null
                                            && entry.video === true
                                            && entry.playable === true
                                            && !loading
                                            && !playing
    readonly property int videoBadgeRadius: videoBadgeSize >= 22
                                             ? Math.round(videoBadgeSize / 2)
                                             : theme.radiusSmall
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
            // MultiEffect 读取隐藏的源图，避免原图在圆角遮罩外露出来。
            visible: false

            onStatusChanged: {
                if (status === Image.Error)
                    control.clearAndFallback()
            }
        }

        // Rectangle.clip 只能做矩形裁剪；用透明底 + 白色圆角层生成真正的 alpha 遮罩。
        Item {
            id: roundedMask
            objectName: "thumbnailMask"

            anchors.fill: parent
            visible: false
            layer.enabled: true

            Rectangle {
                objectName: "thumbnailMaskShape"

                anchors.fill: parent
                radius: control.radius
                color: "white"
            }
        }

        MultiEffect {
            id: previewEffect
            objectName: "thumbnailEffect"

            anchors.fill: parent
            source: preview
            visible: control.imageReady
            autoPaddingEnabled: false
            maskEnabled: true
            maskSource: roundedMask
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

    // 播放中只加一圈内侧强调色，不遮住画面，也不改变缩略图槽尺寸。
    Rectangle {
        objectName: "playingRing"

        anchors.fill: parent
        anchors.margins: 1
        visible: control.playing
        radius: Math.max(0, control.radius - 1)
        color: control.theme.surfaceClear
        border.width: control.playing ? 2 : 0
        border.color: control.theme.accent
    }

    Rectangle {
        objectName: "videoBadge"

        visible: control.showVideoBadge
        width: control.videoBadgeSize
        height: control.videoBadgeSize
        radius: control.videoBadgeRadius
        anchors.right: parent.right
        anchors.rightMargin: control.videoBadgeMargin
        anchors.bottom: parent.bottom
        anchors.bottomMargin: control.videoBadgeMargin
        color: control.theme.mediaBadgeSoft
        border.width: 1
        border.color: Qt.rgba(control.theme.mediaForeground.r,
                              control.theme.mediaForeground.g,
                              control.theme.mediaForeground.b, 0.72)

        Item {
            width: 8
            height: 10
            anchors.centerIn: parent
            scale: control.videoBadgeSize / 20

            Shape {
                anchors.fill: parent

                ShapePath {
                    fillColor: "transparent"
                    strokeColor: control.theme.mediaForeground
                    strokeWidth: 1.1
                    capStyle: ShapePath.RoundCap
                    joinStyle: ShapePath.RoundJoin

                    PathSvg {
                        path: "M1 1 L8 5 L1 9 Z"
                    }
                }
            }
        }
    }
}
