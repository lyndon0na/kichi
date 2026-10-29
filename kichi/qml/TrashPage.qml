import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 回收站表格：查看 / 批量还原 / 彻底删除 / 清空。
// 普通文件页不暴露彻底删除，危险操作只能从这里进入。
Item {
    id: page
    property var backend: null

    FileSelection {
        id: selection
        objectName: "trashSelection"
    }
    FileListDensity { id: density }
    UiMetrics { id: metrics }
    KTheme { id: theme }

    readonly property var entries: backend ? backend.trashFiles : []
    readonly property int selectedCount: selection.count
    readonly property bool busy: backend !== null && (backend.trashLoading || backend.fileActionRunning)

    function selectedEntry() {
        const ids = selection.idList()
        if (ids.length === 0)
            return null
        for (let i = 0; i < entries.length; i++) {
            if (entries[i].id === ids[0])
                return entries[i]
        }
        return null
    }

    function restoreSelected() {
        if (!backend || busy || selection.count === 0)
            return
        backend.untrashEntries(selection.idList())
    }

    function deleteSelected() {
        if (!backend || busy || selection.count === 0)
            return
        const entry = selectedEntry()
        if (!entry)
            return
        dialogs.openDelete(entry, selection.idList())
    }

    function clearAll() {
        if (!backend || busy || entries.length === 0)
            return
        dialogs.openClear(entries.length)
    }

    function openContext(point) {
        contextMenu.popup(point)
    }

    function handleKey(event) {
        if (!backend)
            return
        if (event.key === Qt.Key_A && (event.modifiers & Qt.ControlModifier)) {
            selection.selectAll(entries)
            event.accepted = true
            return
        }
        if (event.key === Qt.Key_Delete) {
            deleteSelected()
            event.accepted = true
            return
        }
        if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
            restoreSelected()
            event.accepted = true
        }
    }

    onVisibleChanged: {
        if (!visible)
            return
        if (backend)
            backend.refreshTrash()
        Qt.callLater(function () { trashList.forceActiveFocus() })
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.topMargin: metrics.pageMargin
        spacing: 8

        RowLayout {
            Layout.fillWidth: true
            Layout.leftMargin: metrics.pageMargin
            Layout.rightMargin: metrics.pageMargin + metrics.scrollBarSlot
            Layout.preferredHeight: 34
            spacing: 8

            Label {
                objectName: "trashSelectionSummary"
                Layout.fillWidth: true
                text: page.selectedCount > 0
                      ? qsTr("已选择 %1 项").arg(page.selectedCount)
                      : qsTr("共 %1 项").arg(page.entries.length)
                color: page.selectedCount > 0 ? theme.textPrimary : theme.textMuted
                font.pixelSize: theme.fontBody
                verticalAlignment: Text.AlignVCenter
            }

            KIconButton {
                iconName: "refresh"
                iconSize: 14
                iconOpacity: 0.65
                enabled: !page.busy
                ToolTip.visible: hovered
                ToolTip.text: qsTr("刷新")
                onClicked: page.backend.refreshTrash()
            }

            KButton {
                objectName: "trashRestoreButton"
                text: qsTr("还原")
                iconName: "restore"
                variant: "primary"
                enabled: page.selectedCount > 0 && !page.busy
                onClicked: page.restoreSelected()
            }

            KButton {
                objectName: "trashDeleteButton"
                text: qsTr("彻底删除")
                iconName: "trash"
                variant: "dangerOutline"
                enabled: page.selectedCount > 0 && !page.busy
                onClicked: page.deleteSelected()
            }

            KButton {
                objectName: "trashClearButton"
                text: qsTr("清空回收站")
                variant: "dangerOutline"
                enabled: page.entries.length > 0 && !page.busy
                onClicked: page.clearAll()
            }
        }

        Label {
            objectName: "trashError"
            Layout.fillWidth: true
            Layout.leftMargin: metrics.pageMargin
            Layout.rightMargin: metrics.pageMargin + metrics.scrollBarSlot
            visible: text.length > 0
            text: page.backend ? page.backend.trashError : ""
            color: theme.danger
            font.pixelSize: theme.fontMeta
            wrapMode: Text.Wrap
        }

        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true

            ColumnLayout {
                anchors.fill: parent
                spacing: 4

                TrashTableHeader {
                    Layout.fillWidth: true
                    iconColumnWidth: density.iconColumnWidth
                    deletedColumnWidth: density.modifiedColumnWidth
                    sizeColumnWidth: density.sizeColumnWidth
                    contentPadding: density.rowPadding
                    columnSpacing: density.columnSpacing
                    headerRowHeight: density.headerRowHeight
                    headerPixelSize: density.metaPixelSize
                }

                ListView {
                    id: trashList
                    objectName: "trashList"

                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    clip: true
                    spacing: 2
                    model: page.entries
                    boundsBehavior: Flickable.StopAtBounds
                    ScrollBar.vertical: KScrollBar {}
                    focus: true
                    Component.onCompleted: forceActiveFocus()
                    onVisibleChanged: {
                        if (visible)
                            forceActiveFocus()
                    }
                    Keys.onPressed: function (event) {
                        page.handleKey(event)
                    }

                    delegate: TrashRow {
                        id: row
                        width: ListView.view.width - metrics.scrollBarSlot
                        entry: modelData
                        rowIndex: index
                        highlighted: !!entry && selection.isSelected(entry.id)
                        iconColumnWidth: density.iconColumnWidth
                        deletedColumnWidth: density.modifiedColumnWidth
                        sizeColumnWidth: density.sizeColumnWidth
                        columnSpacing: density.columnSpacing
                        contentPadding: density.rowPadding
                        rowHeight: density.rowHeight
                        textPixelSize: density.namePixelSize
                        metaPixelSize: density.metaPixelSize
                        iconPixelSize: density.iconPixelSize

                        onSelectRequested: function (modifiers, index) {
                            if (index < 0 || index >= page.entries.length)
                                return
                            selection.click(page.entries[index].id, index, modifiers, page.entries)
                        }

                        onContextMenu: function (point) {
                            if (!entry)
                                return
                            selection.ensureSelected(entry)
                            trashList.forceActiveFocus()
                            var windowPoint = row.mapToItem(null, point.x, point.y)
                            page.openContext(windowPoint)
                        }
                    }
                }
            }

            BusyIndicator {
                anchors.centerIn: parent
                visible: page.backend && page.backend.trashLoading
                running: visible
                width: 28
                height: 28
            }

            Label {
                objectName: "trashEmptyHint"
                anchors.centerIn: parent
                visible: page.backend && page.backend.trashLoaded
                         && !page.backend.trashLoading
                         && page.entries.length === 0
                         && page.backend.trashError.length === 0
                text: qsTr("回收站是空的")
                color: theme.textMuted
                font.pixelSize: theme.fontBody
            }

            Label {
                anchors.centerIn: parent
                visible: page.backend && !page.backend.trashLoaded
                         && !page.backend.trashLoading
                         && page.backend.trashError.length > 0
                text: qsTr("回收站读取失败\n请点刷新重试")
                horizontalAlignment: Text.AlignHCenter
                color: theme.textMuted
                font.pixelSize: theme.fontBody
            }
        }
    }

    Menu {
        id: contextMenu
        objectName: "trashContextMenu"
        parent: Overlay.overlay
        width: 190

        MenuItem {
            text: qsTr("还原")
            enabled: page.selectedCount > 0 && !page.busy
            onTriggered: page.restoreSelected()
        }

        MenuItem {
            text: qsTr("彻底删除")
            enabled: page.selectedCount > 0 && !page.busy
            onTriggered: page.deleteSelected()
        }
    }

    TrashActionDialogs {
        id: dialogs
        backend: page.backend
    }

    Connections {
        target: page.backend

        function onChanged() {
            if (!page.backend)
                return
            selection.prune(page.entries)
        }

        function onFileActionDone(action, ok) {
            if (ok && (action === "untrash" || action === "trashDelete" || action === "trashClear"))
                selection.clear()
        }
    }
}
