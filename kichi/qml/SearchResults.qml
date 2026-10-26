import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 搜索结果（全盘扫描时显示，数据全部来自 backend 的 search* 属性）。
//
// 放在 FilesPage 的列表框里：搜索视图和当前目录共用同一块区域，
// 换视图时不会跳高矮。条目行与目录列表保持一致，额外多一列「所在文件夹」。
Item {
    id: root

    property var backend: null

    // 扫描中：状态行旁边转圈
    readonly property bool scanning: root.backend !== null
                                     && !root.backend.searchSettled
                                     && root.backend.searchActive

    // 空状态的排查建议只在「扫描结束 + 一条都没命中」时出现：
    // 还在扫的时候说「没找到」会误导用户。
    readonly property bool emptyAndSettled: root.backend !== null
                                            && root.backend.searchSettled
                                            && root.backend.searchResults.length === 0

    ColumnLayout {
        anchors.fill: parent
        spacing: 6

        // —— 状态行（扫描中多一个「停止」）——
        RowLayout {
            Layout.fillWidth: true
            Layout.margins: 8
            spacing: 8

            BusyIndicator {
                running: root.scanning
                visible: running
                Layout.preferredWidth: 20
                Layout.preferredHeight: 20
            }

            Label {
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
            Button {
                text: qsTr("停止")
                visible: root.scanning
                onClicked: root.backend.stopSearch()
            }
        }

        // —— 结果表 ——
        ListView {
            id: results

            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.margins: 8
            clip: true
            model: root.backend ? root.backend.searchResults : []
            boundsBehavior: Flickable.StopAtBounds
            ScrollBar.vertical: ScrollBar {
                policy: ScrollBar.AsNeeded
            }

            delegate: ItemDelegate {
                id: row
                width: results.width
                // 两行：名字一行、所在文件夹一行。ItemDelegate 的内容区
                // 默认有上下留白，固定高度会把第二行挤到外面重叠
                height: Math.max(48, contentItem.implicitHeight + 16)
                // 与目录列表一样关掉委托自带的图标渲染
                icon.name: ""
                icon.source: ""
                icon.width: 0
                icon.height: 0
                icon.color: "transparent"

                onClicked: {
                    // 只有文件夹能「进入」；文件走行尾的播放 / 下载
                    if (root.backend && modelData.kind === "folder")
                        root.backend.openSearchHit(modelData.id)
                }

                contentItem: ColumnLayout {
                    spacing: 2

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 10

                        Label {
                            text: root.iconFor(modelData)
                            font.pixelSize: 18
                        }

                        Label {
                            text: modelData.name
                            elide: Text.ElideRight
                            Layout.fillWidth: true
                        }

                        Label {
                            text: qsTr("违规")
                            visible: modelData.auditFlag
                            color: "#c0392b"
                        }

                        Label {
                            text: modelData.sizeText
                            color: palette.placeholderText
                            horizontalAlignment: Text.AlignRight
                            Layout.preferredWidth: 90
                        }

                        Label {
                            text: modelData.modifiedText
                            visible: modelData.modifiedText.length > 0
                            color: palette.placeholderText
                            horizontalAlignment: Text.AlignRight
                            Layout.preferredWidth: 120
                        }

                        ToolButton {
                            text: qsTr("▶")
                            visible: modelData.playable === true
                            enabled: root.backend !== null
                                     && root.backend.playMenu.length === 0
                            ToolTip.visible: hovered
                            ToolTip.text: qsTr("用 mpv 播放")
                            onClicked: {
                                root.playAnchor = row.mapToItem(root, row.width / 2, row.height)
                                root.playRequested(root.playAnchor)
                                root.backend.requestPlay(modelData.id)
                            }
                        }

                        ToolButton {
                            text: qsTr("⬇")
                            enabled: root.backend !== null && !root.backend.downloadRunning
                            ToolTip.visible: hovered
                            ToolTip.text: modelData.kind === "folder"
                                          ? qsTr("下载整个文件夹到本地")
                                          : qsTr("下载到本地")
                            onClicked: root.backend.downloadEntry(modelData.id)
                        }
                    }

                    Label {
                        Layout.fillWidth: true
                        text: qsTr("所在文件夹：%1").arg(modelData.folderText)
                        color: palette.placeholderText
                        font.pixelSize: 12
                        elide: Text.ElideMiddle
                    }
                }

                HoverHandler {
                    cursorShape: modelData.kind === "folder" ? Qt.PointingHandCursor : Qt.ArrowCursor
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

    // 「▶」在结果行里的位置（页面坐标系）：清晰度菜单由父页面从这个点弹出
    property point playAnchor: Qt.point(0, 0)

    signal playRequested(point anchor)

    // 图标按扩展名猜（📁 🎬 🎵 🖼️ 📄）。与 FileRow 同一套规则；
    // 内联而不 `import "...js"`：外部脚本不进 qrc（实测打包后 404）。
    function iconFor(entry) {
        if (!entry)
            return "📄"
        if (entry.kind === "folder")
            return "📁"
        const name = String(entry.name || "").toLowerCase()
        if (/\.(mp4|mkv|avi|mov|webm|ts|m2ts|flv|wmv|rmvb)$/.test(name))
            return "🎬"
        if (/\.(mp3|flac|wav|m4a|ogg|opus|aac|ape)$/.test(name))
            return "🎵"
        if (/\.(jpg|jpeg|png|gif|webp|bmp|heic|avif)$/.test(name))
            return "🖼️"
        return "📄"
    }

}
