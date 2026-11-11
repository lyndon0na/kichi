import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "../components"
import "../core"

// 文件类内容页共用的网格卡片视觉。
//
// 文件、回收站和资料库都复用它。单击选择、右键菜单和双击信号由页面接线；
// 回收站关闭激活，资料库禁止普通文件通过双击下载。
ItemDelegate {
    id: card

    property var backend: null
    required property var entry
    required property int index
    readonly property var gridView: GridView.view
    property int rowIndex: -1
    property bool selected: false
    /// 双击是否执行条目动作；回收站网格关闭这一项，避免误还原 / 误删。
    property bool activationEnabled: true
    /// 双击普通不可播放文件是否直接下载；资料库页只打开或播放，不在这里下载。
    property bool downloadNonPlayable: true
    /// 双击图片是否打开预览；只有文件页开启（资料库 / 回收站没有预览入口）。
    property bool previewImages: false
    /// 当前条目正在取播放直链 / 等待播放器窗口
    property bool playLoading: false
    /// 播放器已经真正开播、尚未退出
    property bool playing: false
    /// 刚创建成功：短暂铺一层强调色，不改变选择态。
    property bool createdFlash: false
    /// 内部拖拽移动的协调器（只有文件页传入；资料库 / 回收站不传，拖拽随之关闭）
    property var internalDrag: null
    /// 当前卡片是不是本次拖动的落点（文件夹卡片才可能为真）
    readonly property bool dropHovered: card.internalDrag !== null
                                        && card.internalDrag.isHoveredFolder(
                                            card.entry ? card.entry.id : "")
    /// 当前卡片正在被拖走：原地淡出，让用户看清这次带走了哪几项
    readonly property bool dragSource: card.internalDrag !== null
                                       && card.internalDrag.isDragged(
                                           card.entry ? card.entry.id : "")
    /// 按下的条目已经准备好拖动数据（平台拖拽还没超过阈值）
    property bool dragPrepared: false
    /// 拖影抓图超时：不再等图片，允许光标下没有拖影地开始拖
    property bool dragImageTimedOut: false
    /// 本次按压期间真的启动过拖拽：松手后不能让这次拖动顺带选中一张卡片
    property bool dragOccurred: false
    readonly property bool showPlayingState: card.playing && !card.playLoading
    /// 卡片底部的单行说明：普通文件显示大小 / 修改时间，搜索结果显示所在文件夹。
    property string detailText: ""
    property bool detailClickable: false
    /// 回收站条目：标「回收站」，双击不进入 / 不播放 / 不下载
    readonly property bool inTrash: !!card.entry && card.entry.inTrash === true
    readonly property string detailDisplayText: card.compactGridDate(detailText)
    readonly property bool detailVisible: detailDisplayText.length > 0
    readonly property bool detailHovered: detailVisible && detailClickable
                                             && mouseArea.containsMouse
                                             && card.pointerOver(gridDetail,
                                                                 mouseArea.mouseX,
                                                                 mouseArea.mouseY)

    // 数值由 FilesPage 从 FileListDensity 下发，和网格列宽一起计算。
    property int contentPadding: 5
    property int cardSpacing: 7
    property int nameLineHeight: 19
    property int nameBlockHeight: 38
    property int thumbnailRadius: 8
    // 由 FileListDensity 下发；窗口 resize 不改变这两个解码尺寸。
    property int thumbnailDecodeWidth: 512
    property int thumbnailDecodeHeight: 341
    property int textPixelSize: 14
    property int detailPixelSize: 12
    property int detailLineHeight: 17
    property int iconPixelSize: 28
    property int videoBadgeSize: 28
    property int videoBadgeMargin: 7

    signal contextMenu(point position)
    signal playRequested(string id, point position)
    signal previewRequested(string id)
    signal selectRequested(int modifiers, int index)
    signal folderOpenRequested(string id)
    signal detailRequested()

    readonly property real thumbnailWidth: Math.max(1, width - contentPadding * 2)
    readonly property int thumbnailHeight: Math.max(1, Math.ceil(thumbnailWidth * 2 / 3))
    readonly property int cardHeight: contentPadding * 2 + thumbnailHeight + cardSpacing
                                      + nameBlockHeight
                                      + (detailVisible ? detailLineHeight : 0)

    leftPadding: contentPadding
    rightPadding: contentPadding
    topPadding: contentPadding
    bottomPadding: contentPadding
    implicitHeight: cardHeight

    // 拖动期间源卡片原地淡出，和列表行同一套「被拿起」反馈。
    opacity: card.dragSource ? 0.45 : 1.0

    Behavior on opacity {
        NumberAnimation {
            duration: theme.motionFast
            easing: Easing.OutCubic
        }
    }

    icon.name: ""
    icon.source: ""
    icon.width: 0
    icon.height: 0
    icon.color: "transparent"

    KTheme { id: theme }

    background: Rectangle {
        radius: theme.radiusCard
        color: card.selected
               ? theme.accentSoft
               : mouseArea.pressed
                 ? theme.surfacePressed
                 : mouseArea.containsMouse
                   ? theme.surfaceHover
                   : card.showPlayingState
                     ? theme.accentGhost
                     : theme.surfaceClear

        Rectangle {
            objectName: "gridCreatedFlash"
            anchors.fill: parent
            radius: parent.radius
            color: theme.accentGhost
            opacity: card.createdFlash ? 1 : 0
            visible: opacity > 0

            Behavior on opacity {
                NumberAnimation {
                    duration: theme.motionFast
                    easing: Easing.OutCubic
                }
            }
        }
    }

    // 拖影抓图超时兜底：抓不到图也必须能拖，只是这次没有光标拖影。
    Timer {
        id: dragImageTimer

        objectName: "gridDragImageTimer"
        interval: 200
        repeat: false
        onTriggered: card.dragImageTimedOut = true
    }

    MouseArea {
        id: mouseArea

        objectName: "gridItemMouseArea"
        anchors.fill: parent
        z: 1
        acceptedButtons: Qt.LeftButton | Qt.RightButton
        hoverEnabled: true
        // 拖动源：只有按下时准备好了数据才挂 target；超过阈值后 MouseArea 才
        // 真正进入拖动，Drag.Automatic 随即启动平台拖拽（见 dragHandle）。
        drag.target: card.dragPrepared ? dragHandle : null
        // 拖影要到下一帧才抓得出来：图没就绪前先把阈值拉高，免得平台拖拽
        // 先启动、光标下空着；超时（抓不到图）后照常放行。
        drag.threshold: card.dragPrepared && card.internalDrag
                        && card.internalDrag.dragPreview !== null
                        && card.internalDrag.dragImageUrl.length === 0
                        && !card.dragImageTimedOut ? 100000 : 8
        cursorShape: card.detailHovered
                     || (!card.inTrash && card.activationEnabled && card.entry
                         && (card.entry.kind === "folder"
                             || card.entry.playable === true
                             || (card.previewImages && card.entry.image === true)
                             || card.downloadNonPlayable))
                     ? Qt.PointingHandCursor : Qt.ArrowCursor

        onPressed: function (mouse) {
            card.dragOccurred = false
            card.dragImageTimedOut = false
            card.dragPrepared = mouse.button === Qt.LeftButton
                    && card.internalDrag !== null
                    && card.internalDrag.begin(card.entry)
            if (card.dragPrepared)
                dragImageTimer.start()
        }

        onReleased: {
            dragImageTimer.stop()
            card.dragPrepared = false
            if (card.internalDrag)
                card.internalDrag.finishDrag()
        }

        onCanceled: {
            dragImageTimer.stop()
            card.dragPrepared = false
            card.dragOccurred = false
            if (card.internalDrag)
                card.internalDrag.finishDrag()
        }

        onClicked: function (mouse) {
            // 拖动结束后平台可能仍补一个 click；它不该改变选择。
            if (card.dragOccurred) {
                card.dragOccurred = false
                return
            }
            if (card.gridView)
                card.gridView.forceActiveFocus()
            if (mouse.button === Qt.RightButton) {
                if (card.entry)
                    card.contextMenu(Qt.point(mouse.x, mouse.y))
                return
            }
            if (card.detailHovered) {
                card.detailRequested()
                return
            }
            card.selectRequested(mouse.modifiers, card.rowIndex)
        }

        onDoubleClicked: function (mouse) {
            if (card.dragOccurred) {
                card.dragOccurred = false
                return
            }
            if (mouse.button === Qt.LeftButton && !card.detailHovered)
                card.activate(Qt.point(mouse.x, mouse.y))
        }
    }

    // 拖动源本体：卡片不能跟着指针走，所以 MouseArea 的 drag.target 指向这个
    // 不可见 1×1 项；Drag.Automatic 在越过阈值时用 mimeData 启动平台拖拽。
    Item {
        id: dragHandle

        objectName: "gridDragHandle"
        width: 1
        height: 1
        opacity: 0
        Drag.active: mouseArea.drag.active
        Drag.dragType: Drag.Automatic
        Drag.supportedActions: Qt.MoveAction
        Drag.proposedAction: Qt.MoveAction
        Drag.source: card
        Drag.mimeData: card.dragPrepared && card.internalDrag
                       ? card.internalDrag.mimeData : ({})
        // 拖影：按下瞬间抓好的小卡片图。没有图也能拖，Cursor 下就是空的。
        Drag.imageSource: card.dragPrepared && card.internalDrag
                          ? card.internalDrag.dragImageUrl : ""
        Drag.hotSpot: Qt.point(24, 24)
        Drag.onDragStarted: {
            card.dragOccurred = true
            if (card.internalDrag)
                card.internalDrag.dragStarted()
        }
        Drag.onDragFinished: function (dropAction) {
            if (card.internalDrag)
                card.internalDrag.finishDrag()
        }
    }

    // 落点：文件夹卡片接受本应用的内部拖拽；文件 / 临时 / 写操作中的卡片不接受。
    DropArea {
        objectName: "gridDropArea"
        anchors.fill: parent
        keys: card.internalDrag ? [card.internalDrag.mimeType] : []
        enabled: card.internalDrag !== null
                 && card.internalDrag.folderTarget(card.entry).length > 0
        onEntered: function (drag) {
            drag.accepted = card.internalDrag !== null
                    && card.internalDrag.hoverFolder(card.entry)
        }
        onExited: {
            if (card.internalDrag)
                card.internalDrag.clearHover()
        }
        onDropped: function (drop) {
            if (card.internalDrag)
                card.internalDrag.dropOnFolder(
                            card.entry,
                            drop.getDataAsString(card.internalDrag.mimeType))
        }
    }

    // 悬停落点高亮：与列表行同一套强调色语言，松手前让用户看清会移到哪里。
    Rectangle {
        objectName: "gridDropHighlight"
        anchors.fill: parent
        radius: theme.radiusCard
        color: theme.accentSoft
        border.width: 2
        border.color: theme.accent
        visible: card.dropHovered
        z: 3
    }

    // 盖住整块缩略图的中央加载层。放在委托根上而不是 contentItem 里：
    // 整卡 MouseArea 的 z=1 会吃掉缩略图内部控件的点击，取消热区必须再高一层。
    VideoLoadingOverlay {
        objectName: "gridVideoLoading"

        x: card.leftPadding
        y: card.topPadding
        width: card.thumbnailWidth
        height: card.thumbnailHeight
        cornerRadius: card.thumbnailRadius
        visible: card.playLoading
        z: 2
        onCancelRequested: {
            if (card.backend)
                card.backend.cancelPlayLoading()
        }
    }

    function activate(point) {
        if (!card.activationEnabled || !card.backend || !card.entry)
            return
        // 回收站里的条目不是现役层级：只能从右键菜单还原
        if (card.inTrash)
            return
        if (card.entry.kind === "folder") {
            card.folderOpenRequested(card.entry.id)
            return
        }
        if (card.entry.playable === true) {
            card.playRequested(card.entry.id, point)
            return
        }
        if (card.previewImages && card.entry.image === true) {
            card.previewRequested(card.entry.id)
            return
        }
        if (card.downloadNonPlayable)
            card.backend.downloadEntry(card.entry.id)
    }

    /// 惯性期间被 Flickable 吞掉的首击：保留“所在文件夹”链接的单击语义。
    function firstClick(modifiers, x, y) {
        const local = card.gridView ? card.gridView.mapToItem(card, x, y)
                                    : Qt.point(x, y)
        if (card.detailVisible && card.detailClickable
                && card.pointerOver(gridDetail, local.x, local.y)) {
            card.detailRequested()
            return
        }
        card.selectRequested(modifiers, card.rowIndex)
    }

    function pointerOver(item, x, y) {
        const topLeft = item.mapToItem(card, 0, 0)
        return x >= topLeft.x && x <= topLeft.x + item.width
                && y >= topLeft.y && y <= topLeft.y + item.height
    }

    // 网格空间紧，日期只保留两位年份；列表仍显示完整年份。
    function compactGridDate(text) {
        return String(text).replace(/\b(19|20)(\d{2})([-/])/g, "$2$3")
    }

    contentItem: ColumnLayout {
        spacing: 0

        Item {
            id: thumbnailBox
            objectName: "gridThumbnailBox"

            Layout.fillWidth: true
            Layout.preferredHeight: card.thumbnailHeight

            Rectangle {
                objectName: "gridThumbnailFrame"

                anchors.fill: parent
                radius: card.thumbnailRadius
                color: theme.surfaceRaised
                border.width: 1
                border.color: theme.hairline
            }

            FileThumbnail {
                objectName: "gridThumbnail"

                anchors.fill: parent
                radius: card.thumbnailRadius
                backend: card.backend
                entry: card.entry
                decodeWidth: card.thumbnailDecodeWidth
                decodeHeight: card.thumbnailDecodeHeight
                iconPixelSize: card.iconPixelSize
                videoBadgeSize: card.videoBadgeSize
                videoBadgeMargin: card.videoBadgeMargin
                loading: card.playLoading
                playing: card.showPlayingState
            }

            Rectangle {
                id: gridPlayingPill
                objectName: "gridPlayingPill"

                visible: card.showPlayingState
                z: 2
                implicitWidth: gridPlayingContent.implicitWidth + 16
                implicitHeight: 24
                width: implicitWidth
                height: implicitHeight
                radius: height / 2
                anchors.right: parent.right
                anchors.rightMargin: 7
                anchors.bottom: parent.bottom
                anchors.bottomMargin: 7
                color: theme.mediaBadge
                Accessible.name: qsTr("播放中")

                RowLayout {
                    id: gridPlayingContent

                    anchors.centerIn: parent
                    spacing: 5

                    KPlayingBars {
                        objectName: "gridPlayingBars"

                        implicitWidth: 10
                        implicitHeight: 10
                        fillColor: theme.accent
                        running: gridPlayingPill.visible
                    }

                    Label {
                        objectName: "gridPlayingText"

                        text: qsTr("播放中")
                        font.pixelSize: theme.fontCaption
                        font.weight: Font.DemiBold
                        color: theme.mediaForeground
                    }
                }
            }

            // 网格没有整行左侧的 3px 竖条，选中态用同色细环 + 卡片淡底，
            // 保持和列表同一套强调色语言。
            Rectangle {
                objectName: "gridSelectionRing"

                anchors.fill: parent
                anchors.margins: 1
                radius: card.thumbnailRadius
                color: "transparent"
                border.width: card.selected ? 2 : 0
                border.color: theme.accent
            }

            Rectangle {
                id: gridStarBadge
                objectName: "gridStarBadge"
                width: 20
                height: width
                radius: width / 2
                anchors.top: parent.top
                anchors.left: parent.left
                anchors.margins: 6
                z: 2
                // 星标要和内容面板及缩略图都拉开对比，圆底用标准深色遮罩，
                // 星本身使用当前方案的高对比前景，避免跟着强调色融进圆底。
                color: theme.mediaBadge
                visible: card.entry ? card.entry.starred === true : false
                Accessible.name: qsTr("已加星标")

                NavIcon {
                    objectName: "gridStar"
                    anchors.centerIn: parent
                    width: theme.fontCaption + 2
                    height: width
                    name: "star"
                    color: theme.mediaForeground
                    filled: true
                }
            }

            KStatusBadge {
                objectName: "gridAuditBadge"

                anchors.top: parent.top
                anchors.right: parent.right
                anchors.margins: 6
                z: 2
                text: qsTr("违规")
                tone: "danger"
                visible: card.entry ? card.entry.auditFlag === true : false
            }

            KStatusBadge {
                objectName: "gridTrashBadge"

                anchors.left: parent.left
                anchors.bottom: parent.bottom
                anchors.margins: 6
                z: 2
                text: qsTr("回收站")
                tone: "neutral"
                visible: card.inTrash
            }
        }

        Item { Layout.preferredHeight: card.cardSpacing }

        ElidedLabel {
            id: gridName
            objectName: "gridName"

            Layout.fillWidth: true
            Layout.preferredHeight: card.nameBlockHeight
            text: card.entry ? card.entry.name : ""
            font.pixelSize: card.textPixelSize
            font.weight: card.entry && card.entry.kind === "folder"
                         ? Font.DemiBold : Font.Normal
            color: theme.textPrimary
            wrapMode: Text.WrapAnywhere
            maximumLineCount: 2
            elide: Text.ElideRight
            lineHeight: card.nameLineHeight
            lineHeightMode: Text.FixedHeight
            verticalAlignment: Text.AlignTop
            externallyHovered: mouseArea.containsMouse
                               && card.pointerOver(gridName, mouseArea.mouseX,
                                                   mouseArea.mouseY)
        }

        ElidedLabel {
            id: gridDetail
            objectName: "gridDetail"

            visible: card.detailVisible
            Layout.fillWidth: true
            Layout.preferredHeight: card.detailVisible ? card.detailLineHeight : 0
            text: card.detailDisplayText
            font.pixelSize: card.detailPixelSize
            color: card.detailClickable ? theme.accent : theme.textMuted
            maximumLineCount: 1
            elide: Text.ElideMiddle
            verticalAlignment: Text.AlignVCenter
            externallyHovered: card.detailHovered
        }

    }
}
