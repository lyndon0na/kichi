import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

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
    /// 当前条目正在取播放直链 / 等待播放器窗口
    property bool playLoading: false
    /// 播放器已经真正开播、尚未退出
    property bool playing: false
    readonly property bool showPlayingState: card.playing && !card.playLoading
    /// 卡片底部的单行说明：普通文件显示大小 / 修改时间，搜索结果显示所在文件夹。
    property string detailText: ""
    property bool detailClickable: false
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
    property int textPixelSize: 14
    property int detailPixelSize: 12
    property int detailLineHeight: 17
    property int iconPixelSize: 28
    property int videoBadgeSize: 28
    property int videoBadgeMargin: 7

    signal contextMenu(point position)
    signal playRequested(string id, point position)
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
    }

    MouseArea {
        id: mouseArea

        anchors.fill: parent
        z: 1
        acceptedButtons: Qt.LeftButton | Qt.RightButton
        hoverEnabled: true
        cursorShape: card.detailHovered
                     || (card.activationEnabled && card.entry
                         && (card.entry.kind === "folder"
                             || card.entry.playable === true
                             || card.downloadNonPlayable))
                     ? Qt.PointingHandCursor : Qt.ArrowCursor

        onClicked: function (mouse) {
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
            if (mouse.button === Qt.LeftButton && !card.detailHovered)
                card.activate(Qt.point(mouse.x, mouse.y))
        }
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
        if (card.entry.kind === "folder") {
            card.folderOpenRequested(card.entry.id)
            return
        }
        if (card.entry.playable === true) {
            card.playRequested(card.entry.id, point)
            return
        }
        if (card.downloadNonPlayable)
            card.backend.downloadEntry(card.entry.id)
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
                color: theme.mediaBadgeSoft
                visible: card.entry ? card.entry.starred === true : false
                Accessible.name: qsTr("已加星标")

                NavIcon {
                    objectName: "gridStar"
                    anchors.centerIn: parent
                    width: theme.fontCaption + 2
                    height: width
                    name: "star"
                    color: theme.accent
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
