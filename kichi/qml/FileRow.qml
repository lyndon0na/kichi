import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 文件列表的一行（从 FilesPage.qml 抽出来：那里已到行数软上限）。
//
// 单击选中（highlighted 由页面按 id 绑定）；双击：文件夹进入、
// 媒体文件播放、其余文件下载。重命名 / 删除等仍在右键菜单里。
// 条目字段来自 Rust 侧的 entry_map（与搜索结果同一套字段）。
ItemDelegate {
    id: row

    required property var entry
    required property int index
    property var backend: null
    // 列表视图本身。⚠ 只能取在委托根上：`ListView.view` 附加属性只在委托根
    // 有效，写在下层 MouseArea 里永远是 null（子项拿不到），
    // 表现为「点了行，快捷键还是没焦点」。
    readonly property var listView: ListView.view
    /// 在列表里的下标（Shift 区间选择用）
    property int rowIndex: -1
    /// 当前条目正在取播放直链 / 等待播放器窗口
    property bool playLoading: false
    /// 播放器已经真正开播、尚未退出
    property bool playing: false
    readonly property bool showPlayingState: row.playing && !row.playLoading
    /// 可选的第二行说明（搜索结果用它显示所在文件夹）。
    property string detailText: ""
    property bool detailClickable: false
    readonly property bool detailVisible: detailText.length > 0
    readonly property bool detailHovered: detailVisible && detailClickable
                                             && mouseArea.containsMouse
                                             && row.pointerOver(detailLabel,
                                                                mouseArea.mouseX,
                                                                mouseArea.mouseY)
    // 列宽由页面统一给出，与表头（FileListHeader）同一组数值
    property int thumbnailSlotWidth: 36
    property int thumbnailSlotHeight: 24
    property int sizeColumnWidth: 96
    property int modifiedColumnWidth: 150
    property int columnSpacing: 12
    // 行高 / 字号 / 图标大小同样由页面给出（列表密度集中在 FilesPage.qml）
    property int rowHeight: 44
    property int textPixelSize: 14
    // 大小 / 时间用次级字号：和表头一致，也让文件名在一行里有主次
    property int metaPixelSize: 12
    property int iconPixelSize: 16
    property int videoBadgeSize: 16
    property int videoBadgeMargin: 2
    // 水平内边距固定：表头按同一个数值摆位。不依赖控件风格的默认 padding，
    // 否则换风格（这里实测是 8 而不是 Qt 默认的 12）表头就和行错开。
    // 数值由页面从 FileListDensity 下发，与表头共用同一个 contentPadding。
    property int contentPadding: 12
    leftPadding: contentPadding
    rightPadding: contentPadding
    topPadding: 0
    bottomPadding: 0

    KTheme { id: theme }

    // 行高显式写死：ItemDelegate 自己算的是「文字高度 + 上下 padding」，
    // 跟着字号漂，和表头对不齐。这里由页面给一个确定值。
    // 内容仍是 RowLayout 填满整行，Label 默认 Text.AlignVCenter，文字自动居中。
    implicitHeight: row.rowHeight

    // 右键：相对本行的位置（交给页面换算到菜单坐标系）
    signal contextMenu(point position)
    // 双击媒体文件：把条目 id 和行内锚点交给页面（清晰度菜单从这里弹出）
    signal playRequested(string id, point position)
    // 单击：把修饰键（Ctrl / Shift）与行下标交给页面做多选
    signal selectRequested(int modifiers, int index)
    // 双击文件夹：交给页面先记滚动位置再进目录
    signal folderOpenRequested(string id)
    // 点击第二行说明（搜索结果里的「所在文件夹」）
    signal detailRequested()

    icon.name: ""
    icon.source: ""
    icon.width: 0
    icon.height: 0
    icon.color: "transparent"

    // 整行手势。
    //
    // 为什么不用 ItemDelegate 自带的 onClicked / onDoubleClicked：它们拿不到
    // 键盘修饰键，而 Ctrl / Shift 多选必须读 `mouse.modifiers`。TapHandler 的
    // eventPoint 也不带修饰键，所以整行铺一个 MouseArea。
    //
    // 注意：MouseArea 会吃掉按下的鼠标事件，Flickable 不再能用鼠标拖动滚动；
    // 滚轮不受影响（原本 ItemDelegate 也会吃掉按下，行为没有退化）。
    MouseArea {
        id: mouseArea

        anchors.fill: parent
        // 盖在 contentItem 之上，保证整行都可点
        z: 1
        acceptedButtons: Qt.LeftButton | Qt.RightButton
        // 悬停底色要用 containsMouse，必须开 hoverEnabled（默认关闭）
        hoverEnabled: true
        cursorShape: row.detailHovered
                     || (row.entry && row.entry.kind === "folder")
                     ? Qt.PointingHandCursor : Qt.ArrowCursor

        onClicked: function (mouse) {
            if (row.listView)
                row.listView.forceActiveFocus()
            if (mouse.button === Qt.RightButton) {
                if (row.entry)
                    row.contextMenu(Qt.point(mouse.x, mouse.y))
                return
            }
            if (row.detailHovered) {
                row.detailRequested()
                return
            }
            row.selectRequested(mouse.modifiers, row.rowIndex)
        }

        onDoubleClicked: function (mouse) {
            if (mouse.button === Qt.LeftButton && !row.detailHovered)
                row.activate(Qt.point(mouse.x, mouse.y))
        }
    }

    // 列表缩略图太小，放不下文字状态条；转圈覆盖整块缩略图，点击即取消。
    // 悬停提示会说明这个动作，名称后不再额外占一个「取消」按钮。
    VideoLoadingOverlay {
        objectName: "rowVideoLoading"

        x: row.leftPadding
        y: Math.round((row.rowHeight - row.thumbnailSlotHeight) / 2)
        width: row.thumbnailSlotWidth
        height: row.thumbnailSlotHeight
        compact: true
        cornerRadius: 3
        visible: row.playLoading
        z: 2
        onCancelRequested: {
            if (row.backend)
                row.backend.cancelPlayLoading()
        }
    }

    // 双击的实际动作；单击选中由页面处理，不在这里做。
    function activate(point) {
        if (!row.backend || !row.entry)
            return
        if (row.entry.kind === "folder") {
            row.folderOpenRequested(row.entry.id)
            return
        }
        if (row.entry.playable === true) {
            row.playRequested(row.entry.id, point)
            return
        }
        row.backend.downloadEntry(row.entry.id)
    }

    /// 惯性期间被 Flickable 吞掉的首击：保留“所在文件夹”链接的单击语义。
    function firstClick(modifiers, x, y) {
        const local = row.listView ? row.listView.mapToItem(row, x, y)
                                   : Qt.point(x, y)
        if (row.detailVisible && row.detailClickable
                && row.pointerOver(detailLabel, local.x, local.y)) {
            row.detailRequested()
            return
        }
        row.selectRequested(modifiers, row.rowIndex)
    }

    function pointerOver(item, x, y) {
        const topLeft = item.mapToItem(row, 0, 0)
        return x >= topLeft.x && x <= topLeft.x + item.width
                && y >= topLeft.y && y <= topLeft.y + item.height
    }

    // 选中态：强调色只铺一层很淡的底，左侧再点一根 3px 竖条。
    // 不整行铺满 palette.highlight——饱和色块会把文件名的对比度压下去，
    // 而屏幕上真正「选中」的通常只有一行，代价却铺满整行。
    // 平铺行只靠底部分隔线分段，不用卡片圆角和行间空隙。
    background: Rectangle {
        color: row.highlighted
               ? theme.accentSoft
               : mouseArea.pressed
                 ? theme.surfacePressed
                 : mouseArea.containsMouse
                   ? theme.surfaceHover
                   : theme.surfaceClear


        Rectangle {
            objectName: "fileRowAccent"
            visible: row.highlighted
            width: 3
            height: Math.max(0, parent.height - 28)
            radius: 1.5
            anchors.left: parent.left
            anchors.leftMargin: 1
            anchors.verticalCenter: parent.verticalCenter
            color: theme.accent
        }

        Rectangle {
            objectName: "fileRowDivider"
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: 1
            color: theme.divider
        }
    }

    contentItem: RowLayout {
        spacing: row.columnSpacing

        // 固定缩略图槽：回退成文件图标时宽度和左边线也完全不变。
        Item {
            objectName: "thumbnailSlot"
            Layout.preferredWidth: row.thumbnailSlotWidth
            Layout.fillHeight: true

            FileThumbnail {
                objectName: "fileThumbnail"
                width: row.thumbnailSlotWidth
                height: row.thumbnailSlotHeight
                anchors.centerIn: parent
                backend: row.backend
                entry: row.entry
                iconPixelSize: row.iconPixelSize
                videoBadgeSize: row.videoBadgeSize
                videoBadgeMargin: row.videoBadgeMargin
                loading: row.playLoading
                playing: row.showPlayingState
            }
        }

        ColumnLayout {
            id: nameGroup
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 0

            Row {
                id: nameLine

                Layout.fillWidth: true
                Layout.fillHeight: true
                spacing: 4
                readonly property int extraCount: 1
                                                    + (rowPlayingStatus.visible ? 1 : 0)
                                                    + (auditLabel.visible ? 1 : 0)
                readonly property real extraWidth:
                    rowStarSlot.width
                    + (rowPlayingStatus.visible ? rowPlayingStatus.implicitWidth : 0)
                    + (auditLabel.visible ? auditLabel.implicitWidth : 0)
                    + extraCount * spacing

                ElidedLabel {
                    id: nameLabel
                    objectName: "fileNameText"
                    width: Math.min(implicitWidth,
                                    Math.max(0, nameLine.width - nameLine.extraWidth))
                    height: nameLine.height
                    text: row.entry ? row.entry.name : ""
                    font.pixelSize: row.textPixelSize
                    verticalAlignment: Text.AlignVCenter
                    elide: Text.ElideRight
                    color: palette.text
                    externallyHovered: mouseArea.containsMouse
                                       && row.pointerOver(nameLabel, mouseArea.mouseX,
                                                          mouseArea.mouseY)
                }

                Rectangle {
                    id: rowPlayingStatus
                    objectName: "rowPlayingStatus"

                    visible: row.showPlayingState
                    width: implicitWidth
                    implicitWidth: rowPlayingContent.implicitWidth + 10
                    height: 18
                    y: Math.round((nameLine.height - height) / 2)
                    radius: theme.radiusSmall
                    color: theme.accentSoft
                    Accessible.name: qsTr("播放中")

                    RowLayout {
                        id: rowPlayingContent

                        anchors.centerIn: parent
                        spacing: 4

                        KPlayingBars {
                            objectName: "rowPlayingBars"

                            implicitWidth: 10
                            implicitHeight: 10
                            fillColor: theme.accent
                            running: rowPlayingStatus.visible
                        }

                        Label {
                            text: qsTr("播放中")
                            font.pixelSize: theme.fontCaption
                            font.weight: Font.DemiBold
                            color: theme.accent
                        }
                    }
                }

                Label {
                    id: auditLabel
                    text: qsTr("违规")
                    visible: row.entry ? row.entry.auditFlag : false
                    width: visible ? implicitWidth : 0
                    height: nameLine.height
                    font.pixelSize: row.textPixelSize
                    verticalAlignment: Text.AlignVCenter
                    color: theme.danger
                }

                // 星标槽始终占位，切换星标不会让文件名和状态左右跳。
                // 放在状态 / 违规标记之后，保留“播放中”紧贴文件名的既有排布。
                Item {
                    id: rowStarSlot
                    objectName: "fileRowStarSlot"
                    width: theme.fontBody
                    height: nameLine.height
                    Accessible.name: qsTr("已加星标")

                    NavIcon {
                        objectName: "fileRowStar"
                        anchors.centerIn: parent
                        width: theme.fontBody
                        height: theme.fontBody
                        name: "star"
                        color: theme.accent
                        filled: true
                        visible: row.entry ? row.entry.starred === true : false
                    }
                }
            }

            ElidedLabel {
                id: detailLabel
                objectName: "fileRowDetail"

                visible: row.detailVisible
                Layout.fillWidth: true
                Layout.preferredHeight: row.detailVisible ? row.metaPixelSize + 4 : 0
                text: row.detailText
                font.pixelSize: row.metaPixelSize
                verticalAlignment: Text.AlignVCenter
                elide: Text.ElideMiddle
                color: row.detailClickable ? theme.accent : theme.textMuted
                externallyHovered: row.detailHovered
            }
        }

        Label {
            objectName: "fileSizeText"
            text: row.entry ? row.entry.sizeText : ""
            font.pixelSize: row.metaPixelSize
            verticalAlignment: Text.AlignVCenter
            color: palette.placeholderText
            horizontalAlignment: Text.AlignRight
            Layout.preferredWidth: row.sizeColumnWidth
        }

        // 没有修改时间的条目也占住整列宽度：否则这一列会塌掉，
        // 大小列跟着右移，表头就对不齐了
        Label {
            objectName: "fileModifiedText"
            text: row.entry ? row.entry.modifiedText : ""
            font.pixelSize: row.metaPixelSize
            verticalAlignment: Text.AlignVCenter
            color: palette.placeholderText
            horizontalAlignment: Text.AlignRight
            Layout.preferredWidth: row.modifiedColumnWidth
        }
    }
}
