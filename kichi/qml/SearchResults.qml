import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 搜索结果（全盘扫描时显示，数据全部来自 backend 的 search* 属性）。
//
// 放在 FilesPage 的列表框里：搜索视图和当前目录共用同一块区域，
// 换视图时不会跳高矮。条目行与目录列表保持一致，额外多一列「所在文件夹」。
// 单击选中；双击文件夹进入、媒体文件播放、其他文件下载；
// 重命名 / 删除等走右键菜单（由页面弹出）。
Item {
    id: root

    property var backend: null
    // 当前选中的结果 id（本视图内生效；与目录列表的选中互不干扰）
    property string selectedId: ""
    // 正在加载的视频 id：图标位置换成可点击的转圈，点击即取消
    property string playLoadingId: ""
    // 播放器正在播放的文件 id（可同时有多个）
    property var playingIds: []
    // 行高 / 字号 / 列宽由 FilesPage 下发，与目录列表同一套密度
    property int rowHeight: 60
    property int textPixelSize: 16
    property int metaPixelSize: 13
    property int iconPixelSize: 24
    property int sizeColumnWidth: 110
    property int modifiedColumnWidth: 150

    // 页面尺寸常量：滚动条车道（见 KScrollBar / docs/pitfalls.md 第 13 条）
    UiMetrics {
        id: metrics
    }

    KTheme { id: theme }

    // 右键：条目本身 + 结果行内的位置（页面换算到菜单坐标系）
    signal contextMenu(var entry, point position)
    // 双击媒体文件：条目 id + 本视图内的锚点（页面再换算到页面坐标系）
    signal playRequested(string id, point position)

    // 扫描中：状态行旁边转圈
    readonly property bool scanning: root.backend !== null
                                     && !root.backend.searchSettled
                                     && root.backend.searchActive

    // 空状态的排查建议只在「扫描结束 + 一条都没命中」时出现：
    // 还在扫的时候说「没找到」会误导用户。
    readonly property bool emptyAndSettled: root.backend !== null
                                            && root.backend.searchSettled
                                            && root.backend.searchResults.length === 0

    function isPlayLoading(id) {
        return root.playLoadingId.length > 0
                && root.playLoadingId === String(id)
    }

    function isPlaying(id) {
        const target = String(id)
        for (let i = 0; i < root.playingIds.length; i++) {
            if (String(root.playingIds[i]) === target)
                return true
        }
        return false
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 6

        // —— 状态行（扫描中多一个「停止」）——
        RowLayout {
            Layout.fillWidth: true
            Layout.margins: theme.gapMd
            spacing: theme.gapSm

            BusyIndicator {
                running: root.scanning
                visible: running
                Layout.preferredWidth: 20
                Layout.preferredHeight: 20
            }

            ElidedLabel {
                Layout.fillWidth: true
                text: root.backend ? root.backend.searchStatus : ""
                wrapMode: Text.Wrap
                elide: Text.ElideRight
                maximumLineCount: 2
                color: palette.text
            }

            Label {
                visible: root.backend !== null && root.backend.searchTruncated
                text: qsTr("（列表只显示前 500 条，命中数见左）")
                color: palette.placeholderText
                font.pixelSize: 12
            }

            // 扫描中才是「停止」；结束时这里不出现按钮——
            // 想重来就改关键词或再点搜索框（同名按钮并排会让人不知道点哪个）
            KButton {
                text: qsTr("停止")
                visible: root.scanning
                variant: "ghost"
                implicitHeight: 28
                onClicked: root.backend.stopSearch()
            }
        }

        // —— 结果表 ——
        ListView {
            id: results

            Layout.fillWidth: true
            Layout.fillHeight: true
            // 右缘留给滚动条：列表铺到窗口右缘，条画在那 10px 车道里
            Layout.topMargin: 8
            Layout.leftMargin: 8
            Layout.bottomMargin: 8
            clip: true
            // 行间留 2px：圆角悬停底不连成一片（与文件列表一致）
            spacing: 2
            model: root.backend ? root.backend.searchResults : []
            boundsBehavior: Flickable.StopAtBounds
            ScrollBar.vertical: KScrollBar {}

            delegate: ItemDelegate {
                id: row
                width: results.width - metrics.scrollBarSlot
                readonly property bool playLoading: root.isPlayLoading(modelData.id)
                readonly property bool playing: root.isPlaying(modelData.id) && !playLoading
                // 两行：名字一行、所在文件夹一行。ItemDelegate 的内容区
                // 默认有上下留白，固定高度会把第二行挤到外面重叠
                height: Math.max(root.rowHeight, contentItem.implicitHeight + 18)
                // 与目录列表一样关掉委托自带的图标渲染
                icon.name: ""
                icon.source: ""
                icon.width: 0
                icon.height: 0
                icon.color: "transparent"

                // 指针在本行内的位置：双击菜单从鼠标处弹出，和右键菜单一致
                property point pointerPos: cursorHint.hovered
                                          ? cursorHint.point.position
                                          : Qt.point(width / 2, height)

                // 与文件页一致：单击选中；双击进入文件夹 / 播放媒体 / 下载其他文件
                highlighted: root.selectedId.length > 0 && root.selectedId === modelData.id
                onClicked: root.selectedId = modelData.id
                onDoubleClicked: {
                    if (!root.backend || row.playLoading)
                        return
                    if (modelData.kind === "folder") {
                        root.backend.searchOpenFolder(modelData.id)
                    } else if (modelData.playable === true) {
                        // 用鼠标真实位置（与右键菜单一致）；没有悬停时退回行底中点
                        root.playRequested(modelData.id,
                                           row.mapToItem(root, row.pointerPos.x, row.pointerPos.y))
                    } else {
                        root.backend.downloadEntry(modelData.id)
                    }
                }

                background: Rectangle {
                    radius: theme.radiusCard
                    // 选中只铺很淡的强调色 + 左侧竖条（与文件列表同一套语言），
                    // 不整行铺满饱和色，文件名对比度才保得住
                    color: row.highlighted ? theme.accentSoft
                         : row.hovered ? theme.surfaceHover
                         : theme.surfaceClear

                    Rectangle {
                        visible: row.highlighted
                        width: 3
                        height: parent.height - 16
                        radius: 1.5
                        anchors.left: parent.left
                        anchors.leftMargin: 1
                        anchors.verticalCenter: parent.verticalCenter
                        color: theme.accent
                    }
                }

                contentItem: ColumnLayout {
                    spacing: 2

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 10

                        Item {
                            objectName: "searchTypeSlot"

                            Layout.preferredWidth: root.iconPixelSize
                            Layout.preferredHeight: root.iconPixelSize
                            Layout.alignment: Qt.AlignVCenter

                            FileIcon {
                                anchors.fill: parent
                                visible: !row.playLoading
                                kind: modelData.kind
                                fileName: modelData.name
                                color: palette.text
                                opacity: modelData.kind === "folder" ? 0.95 : 0.55
                            }

                            Button {
                                id: searchPlayCancel
                                objectName: "searchPlayCancel"

                                anchors.fill: parent
                                visible: row.playLoading
                                hoverEnabled: true
                                padding: 0
                                Accessible.name: qsTr("取消视频加载")

                                background: Rectangle {
                                    radius: theme.radiusSmall
                                    color: searchPlayCancel.down
                                           ? theme.surfacePressed
                                           : searchPlayCancel.hovered
                                             ? theme.surfaceHover
                                             : theme.surfaceClear
                                }

                                contentItem: KLoadingSpinner {
                                    objectName: "searchPlayLoading"

                                    anchors.centerIn: parent
                                    width: root.iconPixelSize
                                    height: root.iconPixelSize
                                    running: searchPlayCancel.visible
                                    color: theme.textSecondary
                                }

                                ToolTip.visible: hovered
                                ToolTip.text: qsTr("点击取消视频加载")
                                onClicked: {
                                    if (root.backend)
                                        root.backend.cancelPlayLoading()
                                }
                            }
                        }

                        // 类型标签：长列表里一眼分出「文件夹 / 文件」
                        Label {
                            text: modelData.kind === "folder" ? qsTr("文件夹") : qsTr("文件")
                            color: modelData.kind === "folder" ? theme.accent
                                 : theme.textMuted
                            font.pixelSize: root.metaPixelSize
                            font.weight: Font.DemiBold
                        }

                        Item {
                            id: searchNameGroup
                            Layout.fillWidth: true
                            Layout.preferredHeight: Math.max(root.iconPixelSize, 22)
                            Layout.alignment: Qt.AlignVCenter

                            Row {
                                id: searchNameLine

                                anchors.fill: parent
                                spacing: 4
                                readonly property int extraCount:
                                    (searchPlayingStatus.visible ? 1 : 0)
                                    + (searchAuditLabel.visible ? 1 : 0)
                                readonly property real extraWidth:
                                    (searchPlayingStatus.visible
                                     ? searchPlayingStatus.implicitWidth : 0)
                                    + (searchAuditLabel.visible
                                       ? searchAuditLabel.implicitWidth : 0)
                                    + extraCount * spacing

                                ElidedLabel {
                                    id: searchNameLabel
                                    objectName: "searchNameText"
                                    width: Math.min(implicitWidth,
                                                    Math.max(0, searchNameLine.width
                                                                - searchNameLine.extraWidth))
                                    height: searchNameLine.height
                                    text: modelData.name
                                    font.pixelSize: root.textPixelSize
                                    verticalAlignment: Text.AlignVCenter
                                    elide: Text.ElideRight
                                    color: theme.textPrimary
                                }

                                Rectangle {
                                    id: searchPlayingStatus
                                    objectName: "searchPlayingStatus"

                                    visible: row.playing
                                    width: implicitWidth
                                    implicitWidth: searchPlayingContent.implicitWidth + 10
                                    height: 18
                                    y: Math.round((searchNameLine.height - height) / 2)
                                    radius: theme.radiusSmall
                                    color: theme.accentSoft
                                    Accessible.name: qsTr("播放中")

                                    RowLayout {
                                        id: searchPlayingContent

                                        anchors.centerIn: parent
                                        spacing: 4

                                        KPlayingBars {
                                            objectName: "searchPlayingBars"

                                            implicitWidth: 10
                                            implicitHeight: 10
                                            fillColor: theme.accent
                                            running: searchPlayingStatus.visible
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
                                    id: searchAuditLabel
                                    text: qsTr("违规")
                                    visible: modelData.auditFlag
                                    width: visible ? implicitWidth : 0
                                    height: searchNameLine.height
                                    font.pixelSize: root.textPixelSize
                                    verticalAlignment: Text.AlignVCenter
                                    color: theme.danger
                                }
                            }
                        }

                        Label {
                            text: modelData.sizeText
                            font.pixelSize: root.textPixelSize
                            color: theme.textMuted
                            horizontalAlignment: Text.AlignRight
                            Layout.preferredWidth: root.sizeColumnWidth
                        }

                        Label {
                            text: modelData.modifiedText
                            visible: modelData.modifiedText.length > 0
                            font.pixelSize: root.textPixelSize
                            color: theme.textMuted
                            horizontalAlignment: Text.AlignRight
                            Layout.preferredWidth: root.modifiedColumnWidth
                        }

                        // 文件夹：直接「进入」（面包屑变成真实层级）
                        KButton {
                            readonly property bool isFolder: modelData.kind === "folder"
                            text: qsTr("进入")
                            variant: "ghost"
                            visible: isFolder
                            enabled: root.backend !== null
                            showTextToolTip: false
                            implicitHeight: 26
                            leftPadding: 10
                            rightPadding: 10
                            ToolTip.visible: hovered
                            ToolTip.text: qsTr("进入这个文件夹，面包屑会显示它在网盘里的真实位置")
                            onClicked: root.backend.searchOpenFolder(modelData.id)
                        }
                    }

                    // 「所在文件夹」本身也能点：文件夹条目推进去、文件条目揭示
                    ElidedLabel {
                        objectName: "searchFolderText"
                        Layout.fillWidth: true
                        text: qsTr("所在文件夹：%1").arg(modelData.folderText)
                        color: modelData.folderId && modelData.folderId.length > 0
                               ? theme.accent : theme.textMuted
                        font.pixelSize: root.metaPixelSize
                        elide: Text.ElideMiddle
                        externallyHovered: folderPathMouse.containsMouse

                        MouseArea {
                            id: folderPathMouse
                            anchors.fill: parent
                            enabled: !!modelData.folderId && modelData.folderId.length > 0
                            cursorShape: enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
                            onClicked: root.backend.searchRevealFolder(modelData.folderId,
                                                                              modelData.kind === "folder")
                        }
                    }
                }

                HoverHandler {
                    id: cursorHint
                    cursorShape: modelData.kind === "folder" ? Qt.PointingHandCursor : Qt.ArrowCursor
                }

                // 右键菜单：只上报行内坐标，换算交给页面
                TapHandler {
                    acceptedButtons: Qt.RightButton
                    onTapped: function (eventPoint) {
                        root.selectedId = modelData.id
                        root.contextMenu(modelData, eventPoint.position)
                    }
                }
            }

            // —— 空状态（扫描结束才出现排查建议）——
            ColumnLayout {
                anchors.centerIn: parent
                width: Math.min(results.width - 40, 460)
                spacing: 8
                visible: root.emptyAndSettled

                Label {
                    Layout.fillWidth: true
                    horizontalAlignment: Text.AlignHCenter
                    text: qsTr("没有找到名称包含「%1」的条目")
                        .arg(root.backend ? root.backend.searchQuery : "")
                    wrapMode: Text.Wrap
                }

                Label {
                    Layout.fillWidth: true
                    text: qsTr("可以试试：\n· 换更短的关键词（搜索按名字字面包含，不支持 * ? 通配符）\n· 确认文件已上传完成，且不在回收站里\n· 只搜文件名，不搜文件内容")
                    color: palette.placeholderText
                    wrapMode: Text.Wrap
                    font.pixelSize: 12
                }
            }
        }
    }

}
