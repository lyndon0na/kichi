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
                text: qsTr("刷新")
                enabled: page.backend !== null && !page.backend.busy
                onClicked: page.backend.refreshFiles()
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

                    onClicked: {
                        if (page.backend && modelData.kind === "folder")
                            page.backend.openDir(modelData.id)
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

        Label {
            text: qsTr("点视频/音频右侧的「▶」用 mpv 播放；下载、重命名等操作在后续版本加入。")
            color: palette.placeholderText
            font.pixelSize: 12
            wrapMode: Text.Wrap
            Layout.fillWidth: true
        }
    }

    // —— 清晰度菜单（后端取到档位后 playMenu 非空即弹出）——
    Menu {
        id: playMenu

        // 弹出时按锚点定位；关掉就把后端的菜单清空
        function openAt(point) {
            x = Math.max(0, Math.min(point.x - width / 2, page.width - width))
            y = Math.max(0, Math.min(point.y + 4, page.height - height))
            open()
        }

        onClosed: {
            // 用户直接关掉菜单 = 放弃这次播放（不调后端，下次 requestPlay 会覆盖）
        }

        contentItem: Column {
            spacing: 2
            padding: 4

            Label {
                text: page.backend ? page.backend.playTitle : ""
                font.bold: true
                elide: Text.ElideMiddle
                width: Math.min(implicitWidth, 320)
                padding: 6
            }

            Repeater {
                model: page.backend ? page.backend.playMenu : []

                delegate: MenuItem {
                    text: modelData.label
                          + (modelData.reason.length > 0 ? qsTr("（%1）").arg(modelData.reason) : "")
                          + (modelData.original ? qsTr(" · 原文件") : "")
                    enabled: modelData.available === true
                    width: Math.max(240, implicitWidth + 24)
                    onClicked: page.backend.playSelected(index)
                }
            }
        }
    }

    // playMenu 一有内容就弹（后端已经过滤了「全部不可用」的情况）
    Connections {
        target: page.backend

        function onChanged() {
            if (page.backend && page.backend.playMenu.length > 0 && !playMenu.visible)
                playMenu.openAt(page.playAnchor)
        }
    }
}
