pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "../components"
import "../core"

// 分享浏览态：标题、路径、条目列表与当前层的提示。
//
// 选中状态由宿主弹窗持有（进入 / 返回时会清空），这里只负责显示与转发点击，
// 保证「转存」始终只针对当前这一层。
ColumnLayout {
    id: pane

    property var backend: null
    property var selected: ({})

    signal toggleRequested(string id)
    signal enterRequested(string id, string name)
    signal goUpRequested()
    signal toggleAllRequested()

    readonly property bool busy: backend ? backend.shareOpenBusy : false
    readonly property string errorText: backend ? backend.shareOpenError : ""

    KTheme { id: theme }

    spacing: theme.gapSm

    ColumnLayout {
        Layout.fillWidth: true
        spacing: 2

        ElidedLabel {
            objectName: "shareOpenTitle"
            Layout.fillWidth: true
            text: pane.backend && pane.backend.shareOpenTitle.length > 0
                  ? pane.backend.shareOpenTitle : qsTr("未命名分享")
            color: theme.textPrimary
            font.pixelSize: theme.fontSection
            font.weight: Font.DemiBold
            elide: Text.ElideRight
        }

        ElidedLabel {
            objectName: "shareOpenMeta"
            Layout.fillWidth: true
            visible: text.length > 0
            text: {
                if (!pane.backend)
                    return ""
                const owner = pane.backend.shareOpenOwner
                const meta = pane.backend.shareOpenMeta
                return owner.length > 0 ? owner + " · " + meta : meta
            }
            color: theme.textMuted
            font.pixelSize: theme.fontMeta
            elide: Text.ElideRight
        }
    }

    RowLayout {
        Layout.fillWidth: true
        spacing: theme.gapXs

        KButton {
            objectName: "shareOpenUp"
            text: qsTr("↑ 上一级")
            variant: "secondary"
            implicitHeight: 30
            enabled: pane.backend !== null && pane.backend.shareOpenPath.length > 1
                     && !pane.busy
            onClicked: pane.goUpRequested()
        }

        ElidedLabel {
            objectName: "shareOpenPath"
            Layout.fillWidth: true
            elide: Text.ElideMiddle
            text: pane.pathText()
            color: theme.textSecondary
            font.pixelSize: theme.fontMeta
        }

        KButton {
            objectName: "shareOpenSelectAll"
            text: qsTr("全选")
            variant: "ghost"
            implicitHeight: 30
            enabled: pane.backend !== null && !pane.busy && pane.rowCount() > 0
            onClicked: pane.toggleAllRequested()
        }
    }

    ListView {
        id: list
        objectName: "shareOpenList"
        Layout.fillWidth: true
        Layout.preferredHeight: 300
        clip: true
        spacing: 2
        model: pane.backend ? pane.backend.shareOpenRows : []
        boundsBehavior: Flickable.StopAtBounds
        ScrollBar.vertical: KScrollBar {}

        delegate: ShareOpenEntryRow {
            width: list.width
            selected: pane.selected[id] === true
            onToggleRequested: pane.toggleRequested(id)
            onEnterRequested: pane.enterRequested(id, name)
        }

        Label {
            objectName: "shareOpenEmpty"
            anchors.centerIn: parent
            visible: pane.backend !== null && !pane.busy
                     && list.count === 0 && pane.errorText.length === 0
            text: qsTr("这个文件夹里没有内容")
            color: theme.textMuted
            font.pixelSize: theme.fontBody
        }
    }

    RowLayout {
        visible: pane.busy
        spacing: theme.gapSm

        BusyIndicator {
            Layout.preferredWidth: 18
            Layout.preferredHeight: 18
            running: parent.visible
        }

        Label {
            text: qsTr("正在读取分享内容…")
            color: theme.textMuted
            font.pixelSize: theme.fontMeta
        }
    }

    Label {
        objectName: "shareOpenBrowseError"
        Layout.fillWidth: true
        wrapMode: Text.Wrap
        visible: text.length > 0
        text: pane.errorText
        color: theme.danger
        font.pixelSize: theme.fontMeta
    }

    Label {
        objectName: "shareOpenTruncated"
        Layout.fillWidth: true
        wrapMode: Text.Wrap
        visible: pane.backend !== null && pane.backend.shareOpenTruncated
        text: qsTr("条目太多，这里只显示了前一部分。")
        color: theme.textMuted
        font.pixelSize: theme.fontCaption
    }

    function rowCount() {
        return pane.backend && pane.backend.shareOpenRows
                ? pane.backend.shareOpenRows.length : 0
    }

    function pathText() {
        const path = pane.backend && pane.backend.shareOpenPath
                     ? pane.backend.shareOpenPath : []
        const names = []
        for (let i = 0; i < path.length; i++)
            names.push(String(path[i].name || ""))
        return names.join(" / ")
    }
}
