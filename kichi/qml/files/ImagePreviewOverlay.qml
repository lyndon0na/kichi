import QtQuick
import QtQuick.Controls
import "../components"
import "../core"

// 图片预览浮层：全窗口深色遮罩 + 原图查看器。
//
// 取图永远由 Rust 完成（详情 → 签名直链 → 本地临时文件），QML 只读
// `backend.imagePreviewPath` 的 file:// URI，直接加载远程 URL 是不允许的。
//
// 交互：滚轮缩放（以指针为中心）、放大后拖动平移、「适应窗口」复位、
// Esc 或右上角关闭。加载 / 失败 / 解码失败都在浮层里说明原因。
Popup {
    id: root

    property var backend: null
    /// 当前预览条目的 id 与名字（名字只用于标题栏）
    property string entryId: ""
    property string entryName: ""
    /// 相对「适应窗口」的缩放倍数与平移量
    property real zoom: 1
    property real panX: 0
    property real panY: 0

    readonly property string imageSource: backend !== null
                                            && typeof backend.imagePreviewPath !== "undefined"
                                            ? String(backend.imagePreviewPath) : ""
    readonly property string rustError: backend !== null
                                        && typeof backend.imagePreviewError !== "undefined"
                                        ? String(backend.imagePreviewError) : ""
    readonly property bool rustLoading: backend !== null
                                        && backend.imagePreviewLoading === true
    readonly property bool decodeFailed: root.imageSource.length > 0
                                         && previewImage.status === Image.Error
    readonly property string errorText: root.rustError.length > 0
                                        ? root.rustError
                                        : root.decodeFailed
                                          ? qsTr("无法解码这张图片，文件可能已损坏或格式不受支持")
                                          : ""
    readonly property bool hasError: root.errorText.length > 0
    readonly property bool imageReady: !root.hasError
                                       && root.imageSource.length > 0
                                       && previewImage.status === Image.Ready
    readonly property bool loading: !root.hasError && !root.imageReady
                                    && (root.rustLoading
                                        || root.imageSource.length === 0
                                        || previewImage.status === Image.Loading
                                        || previewImage.status === Image.Null)

    // 深色媒体界面：主题令牌挂属性、以 background 为 parent（同 KDialog 的写法）
    readonly property KTheme theme: KTheme { parent: root.background }

    parent: Overlay.overlay
    anchors.centerIn: Overlay.overlay
    width: parent ? parent.width : 0
    height: parent ? parent.height : 0
    padding: 0
    modal: true
    focus: true
    closePolicy: Popup.CloseOnEscape

    function openFor(id, name) {
        root.entryId = String(id)
        root.entryName = String(name)
        root.fitToWindow()
        root.open()
        root.forceFocus()
    }

    function dismiss() {
        root.close()
    }

    function forceFocus() {
        if (contentItem)
            contentItem.forceActiveFocus()
    }

    function fitToWindow() {
        root.zoom = 1
        root.panX = 0
        root.panY = 0
    }

    function boundPanX(value) {
        const limit = Math.max(0, (viewport.width * root.zoom - viewport.width) / 2)
        return Math.max(-limit, Math.min(limit, value))
    }

    function boundPanY(value) {
        const limit = Math.max(0, (viewport.height * root.zoom - viewport.height) / 2)
        return Math.max(-limit, Math.min(limit, value))
    }

    /// 以指针位置为中心缩放：指针下的图像内容保持不动。
    function zoomAt(px, py, factor) {
        if (!root.imageReady)
            return
        const previous = root.zoom
        const next = Math.max(1, Math.min(8, previous * factor))
        if (next === previous)
            return
        const cx = px - viewport.width / 2
        const cy = py - viewport.height / 2
        const contentX = (cx - root.panX) / previous
        const contentY = (cy - root.panY) / previous
        root.zoom = next
        root.panX = root.boundPanX(cx - contentX * next)
        root.panY = root.boundPanY(cy - contentY * next)
        if (next === 1) {
            root.panX = 0
            root.panY = 0
        }
    }

    onImageSourceChanged: {
        // 换图 / 新图到达时回到适应窗口，拖动后的位置不带到下一张
        root.fitToWindow()
    }

    onOpened: root.forceFocus()

    onClosed: {
        if (root.backend && typeof root.backend.closeImagePreview === "function")
            root.backend.closeImagePreview()
    }

    background: Rectangle {
        color: theme.mediaSurface
    }

    contentItem: Item {
        id: contentRoot

        focus: true
        Keys.onEscapePressed: root.dismiss()

        // —— 图像视口 ——
        Item {
            id: viewport

            anchors.fill: parent
            anchors.topMargin: 48
            anchors.bottomMargin: 36
            clip: true

            HoverHandler {
                cursorShape: root.zoom > 1
                             ? (dragHandler.active ? Qt.ClosedHandCursor : Qt.OpenHandCursor)
                             : Qt.ArrowCursor
            }

            Item {
                id: imageHolder

                width: Math.max(1, viewport.width * root.zoom)
                height: Math.max(1, viewport.height * root.zoom)
                x: Math.round((viewport.width - width) / 2 + root.panX)
                y: Math.round((viewport.height - height) / 2 + root.panY)

                Image {
                    id: previewImage

                    anchors.fill: parent
                    source: root.imageSource
                    fillMode: Image.PreserveAspectFit
                    asynchronous: true
                    smooth: true
                    cache: false
                    visible: root.imageReady
                    Accessible.name: root.entryName.length > 0
                                     ? qsTr("图片预览：%1").arg(root.entryName)
                                     : qsTr("图片预览")
                }
            }

            WheelHandler {
                acceptedDevices: PointerDevice.Mouse | PointerDevice.TouchPad
                onWheel: function (event) {
                    const delta = event.angleDelta.y !== 0
                                  ? event.angleDelta.y : event.angleDelta.x
                    if (delta === 0)
                        return
                    root.zoomAt(event.x, event.y, delta > 0 ? 1.18 : 1 / 1.18)
                    event.accepted = true
                }
            }

            DragHandler {
                id: dragHandler

                target: null
                enabled: root.imageReady && root.zoom > 1
                property real startPanX: 0
                property real startPanY: 0

                onActiveChanged: {
                    if (active) {
                        startPanX = root.panX
                        startPanY = root.panY
                    }
                }
                onActiveTranslationChanged: {
                    if (!active)
                        return
                    root.panX = root.boundPanX(startPanX + activeTranslation.x)
                    root.panY = root.boundPanY(startPanY + activeTranslation.y)
                }
            }
        }

        // —— 加载态 ——
        Column {
            objectName: "imagePreviewLoading"

            anchors.centerIn: viewport
            visible: root.loading
            spacing: 10

            KLoadingSpinner {
                anchors.horizontalCenter: parent.horizontalCenter
                width: 28
                height: 28
                running: root.loading
                color: root.theme.mediaForeground
            }

            Label {
                text: qsTr("正在加载图片…")
                font.pixelSize: root.theme.fontBody
                color: root.theme.mediaText
            }
        }

        // —— 失败态：原因可读，不静默 ——
        Column {
            objectName: "imagePreviewError"

            anchors.centerIn: viewport
            width: Math.min(viewport.width - 48, 520)
            visible: root.hasError
            spacing: 10

            Label {
                width: parent.width
                horizontalAlignment: Text.AlignHCenter
                text: root.errorText
                wrapMode: Text.Wrap
                font.pixelSize: root.theme.fontBody
                color: root.theme.mediaText
            }

            Label {
                width: parent.width
                horizontalAlignment: Text.AlignHCenter
                text: qsTr("按 Esc 或点右上角关闭")
                font.pixelSize: root.theme.fontCaption
                color: root.theme.mediaTextMuted
            }
        }

        // —— 顶部标题栏 ——
        Item {
            id: header

            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            height: 48

            ElidedLabel {
                objectName: "imagePreviewTitle"

                anchors.left: parent.left
                anchors.leftMargin: 16
                anchors.right: headerActions.left
                anchors.rightMargin: 12
                anchors.verticalCenter: parent.verticalCenter
                text: root.entryName.length > 0 ? root.entryName : qsTr("图片预览")
                font.pixelSize: root.theme.fontBody
                font.weight: Font.DemiBold
                color: root.theme.mediaText
                elide: Text.ElideMiddle
            }

            Row {
                id: headerActions

                anchors.right: parent.right
                anchors.rightMargin: 10
                anchors.verticalCenter: parent.verticalCenter
                spacing: 2

                Label {
                    objectName: "imagePreviewZoomLabel"

                    anchors.verticalCenter: parent.verticalCenter
                    visible: root.imageReady && root.zoom > 1
                    text: Math.round(root.zoom * 100) + "%"
                    font.pixelSize: root.theme.fontCaption
                    color: root.theme.mediaTextMuted
                }

                KIconButton {
                    id: fitButton

                    objectName: "imagePreviewFitButton"

                    iconName: "maximize"
                    iconSize: 16
                    iconColor: root.theme.mediaText
                    iconOpacity: 0.9
                    onMedia: true
                    implicitWidth: 32
                    implicitHeight: 32
                    Accessible.name: qsTr("适应窗口")
                    KMediaToolTip {
                        visible: fitButton.hovered
                        text: qsTr("适应窗口")
                    }
                    onClicked: root.fitToWindow()
                }

                KIconButton {
                    id: closeButton

                    objectName: "imagePreviewCloseButton"

                    iconName: "x"
                    iconSize: 16
                    iconColor: root.theme.mediaText
                    iconOpacity: 0.9
                    onMedia: true
                    implicitWidth: 32
                    implicitHeight: 32
                    Accessible.name: qsTr("关闭预览")
                    KMediaToolTip {
                        visible: closeButton.hovered
                        text: qsTr("关闭预览（Esc）")
                    }
                    onClicked: root.dismiss()
                }
            }
        }

        // —— 底部操作提示 ——
        Label {
            objectName: "imagePreviewHint"

            anchors.bottom: parent.bottom
            anchors.bottomMargin: 10
            anchors.horizontalCenter: parent.horizontalCenter
            visible: root.imageReady
            text: qsTr("滚轮缩放 · 拖动平移 · Esc 关闭")
            font.pixelSize: root.theme.fontCaption
            color: root.theme.mediaTextMuted
        }
    }
}
