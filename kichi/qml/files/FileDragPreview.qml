import QtQuick
import QtQuick.Controls
import "../components"
import "../core"

// 内部拖拽跟随指针的「拖影」。
//
// Linux 上 Drag.Automatic 不抓取源条目，光标下默认什么都没有。这里画一张
// 固定尺寸的小卡片，由 FileInternalDrag 在按下的那一帧 grabToImage 成图片，
// 交给 Drag.imageSource 作为平台拖影——用户能看到自己「拿起了」这一条。
//
// 尺寸必须小：拖影太大就会盖住底下的落点。多选时背后露出一张卡、右侧显示
// 「N 项」，接近系统文件管理器把一摞文件托起来的样子。
Item {
    id: preview

    property var entry: null
    property int itemCount: 1

    KTheme { id: theme }

    // 比主卡多出 6px：多选时背后那一层从右下方露出来。
    width: 214
    height: 70

    // 多选时背后露出的一层：抓图时一起成像，看起来像一摞。
    Rectangle {
        objectName: "dragPreviewStack"

        visible: preview.itemCount > 1
        x: 6
        y: 6
        width: 208
        height: 64
        radius: theme.radiusCard
        color: theme.surfaceRaised
        border.width: 1
        border.color: theme.hairline
    }

    Rectangle {
        id: mainCard

        width: 208
        height: 64
        radius: theme.radiusCard
        color: theme.surfaceRaised
        border.width: 1
        border.color: theme.accent

        FileIcon {
            objectName: "dragPreviewIcon"

            x: 14
            y: Math.round((mainCard.height - height) / 2)
            width: 28
            height: 28
            kind: preview.entry ? preview.entry.kind : ""
            fileName: preview.entry ? preview.entry.name : ""
            color: theme.textPrimary
        }

        ElidedLabel {
            objectName: "dragPreviewName"

            x: 52
            y: 12
            width: preview.itemCount > 1 ? 96 : 138
            text: preview.entry ? preview.entry.name : ""
            font.pixelSize: theme.fontBody
            color: theme.textPrimary
            elide: Text.ElideMiddle
            maximumLineCount: 1
        }

        // 单条时补一行大小，多选时把位置让给「N 项」标记。
        Label {
            objectName: "dragPreviewDetail"

            x: 52
            y: 34
            visible: preview.itemCount === 1 && text.length > 0
            text: preview.entry ? String(preview.entry.sizeText || "") : ""
            font.pixelSize: theme.fontCaption
            color: theme.textMuted
        }

        KStatusBadge {
            objectName: "dragPreviewCount"

            visible: preview.itemCount > 1
            anchors.right: parent.right
            anchors.rightMargin: 10
            anchors.verticalCenter: parent.verticalCenter
            text: qsTr("%1 项").arg(preview.itemCount)
            tone: "accent"
        }
    }
}
