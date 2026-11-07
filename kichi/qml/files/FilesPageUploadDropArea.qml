import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "../core"
import "../components"

// 文件页的外部拖放入口。DropArea 只负责接住 text/uri-list，
// 本地文件 / 文件夹解析、特殊项检查和去重都交给 Rust，和 Portal 入口共用规则。
Item {
    id: root

    objectName: "uploadDropRoot"
    property var backend: null
    property bool acceptsDrops: false
    /// 拖入时所在的云端目录；根目录为空串
    property string parentId: ""
    property string destinationName: ""

    KTheme { id: theme }

    function submitUrls(urls) {
        if (!root.acceptsDrops || !root.backend
                || typeof root.backend.uploadDroppedFiles !== "function"
                || !urls || urls.length === 0)
            return
        const accepted = []
        for (let i = 0; i < urls.length; i++) {
            const value = String(urls[i] || "")
            if (value.length > 0)
                accepted.push(value)
        }
        if (accepted.length > 0)
            root.backend.uploadDroppedFiles(root.parentId, root.destinationName,
                                            accepted)
    }

    DropArea {
        id: dropArea

        objectName: "uploadDropArea"
        anchors.fill: parent
        enabled: root.acceptsDrops
        keys: ["text/uri-list"]

        onEntered: function (drag) {
            drag.accepted = root.acceptsDrops && drag.hasUrls
        }
        onDropped: function (drop) {
            if (!root.acceptsDrops || !drop.hasUrls)
                return
            drop.acceptProposedAction()
            root.submitUrls(drop.urls)
        }
    }

    Rectangle {
        objectName: "uploadDropOverlay"
        anchors.fill: parent
        visible: root.acceptsDrops && dropArea.containsDrag
        // 拖入时把底下的文件卡片压下去，避免提示文字和网格内容叠在一起。
        color: Qt.rgba(theme.surface.r, theme.surface.g, theme.surface.b,
                       theme.darkMode ? 0.90 : 0.92)
        border.width: 2
        border.color: theme.accent
        radius: theme.radiusSurface

        Rectangle {
            id: card

            objectName: "uploadDropCard"
            anchors.centerIn: parent
            width: Math.min(root.width - 48, 520)
            height: prompt.implicitHeight + 48
            color: theme.surfaceRaised
            border.width: 1
            border.color: theme.hairline
            radius: theme.radiusSurface

            ColumnLayout {
                id: prompt

                anchors.fill: parent
                anchors.margins: 24
                spacing: theme.gapMd

                Rectangle {
                    Layout.alignment: Qt.AlignHCenter
                    implicitWidth: 60
                    implicitHeight: 60
                    radius: 30
                    color: theme.accentSoft

                    NavIcon {
                        anchors.centerIn: parent
                        width: 30
                        height: 30
                        name: "upload"
                        color: theme.accent
                    }
                }

                Label {
                    objectName: "uploadDropTitle"
                    Layout.fillWidth: true
                    text: qsTr("松开以上传或添加离线任务到 %1").arg(
                              root.destinationName || qsTr("当前文件夹"))
                    color: theme.textPrimary
                    font.pixelSize: theme.fontSection
                    font.weight: Font.DemiBold
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.Wrap
                }

                Label {
                    Layout.fillWidth: true
                    text: qsTr("普通文件 / 文件夹会上传；.torrent 会解析并加入离线任务。符号链接和特殊文件会跳过")
                    color: theme.textSecondary
                    font.pixelSize: theme.fontMeta
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.Wrap
                }
            }
        }
    }
}
