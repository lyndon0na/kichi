import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 文件页（S3）：面包屑导航 + 目录列表。
// 只调 backend 的 Q_INVOKABLE；配额沿用顶栏的 quotaSummary，不重复显示。
// 播放（S5）：点行尾「▶」→ backend 取档位 → playMenu 非空时弹清晰度菜单。
Item {
    id: page

    property var backend: null
    // 「▶」按钮在页面坐标系里的位置（菜单从这里弹出）
    property point playAnchor: Qt.point(0, 0)

    function crumbCount() {
        return page.backend ? page.backend.breadcrumb.length : 0
    }

    function isCurrentCrumb(index) {
        return index === page.crumbCount() - 1
    }

    function sortLabel() {
        if (!page.backend)
            return ""
        var field = page.backend.sortField
        var name = field === "size" ? qsTr("大小")
                 : field === "modified" ? qsTr("时间")
                 : qsTr("名称")
        return name + (page.backend.sortAscending ? " ↑" : " ↓")
    }

    // 点一下换字段，再点当前字段则翻方向（与老习惯一致）
    function cycleSort() {
        if (!page.backend)
            return
        var order = ["name", "modified", "size"]
        var index = order.indexOf(page.backend.sortField)
        if (index < 0)
            index = 0
        if (index + 1 < order.length)
            page.backend.setSorting(order[index + 1], page.backend.sortAscending)
        else
            page.backend.setSorting(page.backend.sortField, !page.backend.sortAscending)
    }

    function iconFor(item) {
        if (item.kind === "folder")
            return "📁"
        const name = item.name.toLowerCase()
        if (/\.(mp4|mkv|avi|mov|webm|ts|m2ts|flv|wmv|rmvb)$/.test(name))
            return "🎬"
        if (/\.(mp3|flac|wav|m4a|ogg|opus|aac|ape)$/.test(name))
            return "🎵"
        if (/\.(jpg|jpeg|png|gif|webp|bmp|heic|avif)$/.test(name))
            return "🖼️"
        return "📄"
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 10

        // —— 导航条 ——
        RowLayout {
            Layout.fillWidth: true
            spacing: 6

            ToolButton {
                text: qsTr("↑ 上一级")
                enabled: page.backend !== null && !page.backend.busy && page.crumbCount() > 1
                onClicked: page.backend.openParent()
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: 2

                Repeater {
                    model: page.backend ? page.backend.breadcrumb : []

                    delegate: RowLayout {
                        spacing: 2

                        Label {
                            text: "›"
                            visible: index > 0
                            color: palette.placeholderText
                        }

                        Label {
                            text: modelData
                            font.bold: page.isCurrentCrumb(index)
                            color: page.isCurrentCrumb(index) ? palette.text : palette.link
                            elide: Text.ElideMiddle
                            Layout.maximumWidth: 220

                            MouseArea {
                                anchors.fill: parent
                                enabled: !page.isCurrentCrumb(index)
                                cursorShape: enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
                                onClicked: page.backend.openCrumb(index)
                            }
                        }
                    }
                }
            }

            BusyIndicator {
                running: page.backend !== null && page.backend.busy
                visible: running
                Layout.preferredWidth: 24
                Layout.preferredHeight: 24
            }

            ToolButton {
                text: page.sortLabel()
                enabled: page.backend !== null
                ToolTip.visible: hovered
                ToolTip.text: qsTr("点按切换排序字段（名称 → 时间 → 大小），循环到末尾再点翻转方向")
                onClicked: page.cycleSort()
            }

            ToolButton {
                text: qsTr("刷新")
                enabled: page.backend !== null && !page.backend.busy
                onClicked: page.backend.refreshFiles()
            }

            ToolButton {
                text: qsTr("＋ 新建文件夹")
                enabled: page.backend !== null && !page.backend.busy
                         && !page.backend.fileActionRunning
                onClicked: menu.openNewFolder()
            }
        }

        // —— 列表 ——
        Frame {
            Layout.fillWidth: true
            Layout.fillHeight: true
            padding: 0

            ListView {
                id: fileList

                anchors.fill: parent
                clip: true
                model: page.backend ? page.backend.files : []
                boundsBehavior: Flickable.StopAtBounds
                ScrollBar.vertical: ScrollBar {
                    policy: ScrollBar.AsNeeded
                }

                delegate: ItemDelegate {
                    width: ListView.view.width
                    height: 44
                    // 关掉委托的图标渲染：条目 map 里没有图片源，
                    // 默认行为会把非图片值当 image provider 拉成空白缩略图
                    icon.name: ""
                    icon.source: ""
                    icon.width: 0
                    icon.height: 0
                    icon.color: "transparent"

                    onClicked: {
                        if (page.backend && modelData.kind === "folder")
                            page.backend.openDir(modelData.id)
                    }

                    // 右键：重命名 / 移至回收站
                    TapHandler {
                        acceptedButtons: Qt.RightButton
                        onTapped: function (eventPoint) {
                            if (!page.backend)
                                return
                            menu.openContext(modelData, eventPoint.position)
                        }
                    }

                    contentItem: RowLayout {
                        spacing: 10

                        Label {
                            text: page.iconFor(modelData)
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
                            Layout.preferredWidth: 100
                        }

                        Label {
                            text: modelData.modifiedText
                            visible: modelData.modifiedText.length > 0
                            color: palette.placeholderText
                            horizontalAlignment: Text.AlignRight
                            Layout.preferredWidth: 130
                        }

                        // 只有视频/音频才显示播放入口
                        ToolButton {
                            text: qsTr("▶")
                            visible: modelData.playable === true
                            enabled: page.backend !== null && !page.backend.busy
                                     && page.backend.playMenu.length === 0
                            ToolTip.visible: hovered
                            ToolTip.text: qsTr("用 mpv 播放")

                            onClicked: {
                                page.playAnchor = mapToItem(page, width / 2, height)
                                page.backend.requestPlay(modelData.id)
                            }
                        }

                        // 下载到本地（文件直接下；文件夹递归下，保留层级）
                        ToolButton {
                            text: qsTr("⬇")
                            enabled: page.backend !== null && !page.backend.downloadRunning
                            ToolTip.visible: hovered
                            ToolTip.text: modelData.kind === "folder"
                                          ? qsTr("下载整个文件夹到本地")
                                          : qsTr("下载到本地")

                            onClicked: page.backend.downloadEntry(modelData.id)
                        }
                    }

                    HoverHandler {
                        cursorShape: modelData.kind === "folder" ? Qt.PointingHandCursor : Qt.ArrowCursor
                    }
                }
            }

            Label {
                anchors.centerIn: parent
                text: qsTr("这个文件夹是空的")
                color: palette.placeholderText
                visible: page.backend !== null && !page.backend.busy
                         && page.backend.files.length === 0
            }
        }

        // —— 下载进度（有下载在跑时出现；点 × 取消）——
        DownloadBar {
            Layout.fillWidth: true
            backend: page.backend
        }

        Label {
            text: qsTr("点视频/音频右侧的「▶」用 mpv 播放；点「⬇」下载到本地（文件夹会连同子目录一起下）。右键条目可重命名或移到回收站。下载目录在设置页里改。")
            color: palette.placeholderText
            font.pixelSize: 12
            wrapMode: Text.Wrap
            Layout.fillWidth: true
        }
    }

    // —— 右键菜单与重命名 / 新建 / 删除弹窗（内容在 FilesPageMenu.qml）——
    FilesPageMenu {
        id: menu
        anchors.fill: parent
        backend: page.backend
    }

    // —— 清晰度菜单（内容在 FilesPagePlayMenu.qml）——
    FilesPagePlayMenu {
        id: playMenu
        backend: page.backend
        anchorPoint: page.playAnchor
    }
}
