import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 最近保存 / 播放历史 / 已加星标的公共表格骨架。
// 数据来源、空态文案与两列含义由三个薄页面分别提供。
Item {
    id: page

    property var backend: null
    property string kind: ""
    property string primaryHeader: ""
    property string secondaryHeader: ""
    property string emptyText: ""

    readonly property bool activeCollection: page.backend !== null
                                            && page.backend.libraryKind === page.kind
    readonly property var entries: page.activeCollection ? page.backend.libraryRows : []
    readonly property bool loading: page.activeCollection && page.backend.libraryLoading
    readonly property bool loaded: page.activeCollection && page.backend.libraryLoaded
    readonly property string errorText: page.activeCollection ? page.backend.libraryError : ""
    property int lastRevision: -1

    FileListDensity {
        id: density
        objectName: "libraryDensity"
    }
    UiMetrics { id: metrics }
    KTheme { id: theme }

    function isPlayLoading(id) {
        return page.backend !== null
                && page.backend.playLoading === true
                && String(page.backend.playLoadingId) === String(id)
    }

    function isPlaying(id) {
        const ids = page.backend && page.backend.playingIds
                    ? page.backend.playingIds : []
        for (let i = 0; i < ids.length; ++i) {
            if (String(ids[i]) === String(id))
                return true
        }
        return false
    }

    function refresh() {
        if (!page.backend || page.kind.length === 0)
            return
        page.backend.refreshLibrary(page.kind)
    }

    function playEntry(id, anchorPoint) {
        if (!page.backend)
            return
        playMenu.anchorPoint = anchorPoint
        page.backend.requestPlay(id)
    }

    function activateCurrent() {
        if (!page.backend || entries.length === 0)
            return
        const index = Math.max(0, Math.min(libraryList.currentIndex, entries.length - 1))
        const entry = entries[index]
        if (!entry)
            return
        if (entry.kind === "folder") {
            page.backend.openLibraryEntry(entry.id)
        } else if (entry.playable === true) {
            page.playEntry(entry.id, libraryList.mapToItem(null,
                                                         libraryList.width / 2,
                                                         Math.max(24, libraryList.height / 2)))
        }
    }

    onVisibleChanged: {
        if (!visible)
            return
        page.refresh()
        Qt.callLater(function () { libraryList.forceActiveFocus() })
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.topMargin: metrics.pageMargin
        spacing: 8

        Label {
            objectName: "libraryError"
            Layout.fillWidth: true
            Layout.leftMargin: metrics.pageMargin
            Layout.rightMargin: metrics.pageMargin + metrics.scrollBarSlot
            visible: text.length > 0
            text: page.errorText
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

                LibraryTableHeader {
                    Layout.fillWidth: true
                    thumbnailSlotWidth: density.thumbnailSlotWidth
                    primaryColumnWidth: density.sizeColumnWidth
                    secondaryColumnWidth: density.modifiedColumnWidth
                    contentPadding: density.rowPadding
                    columnSpacing: density.columnSpacing
                    headerRowHeight: density.headerRowHeight
                    headerPixelSize: density.metaPixelSize
                    primaryLabel: page.primaryHeader
                    secondaryLabel: page.secondaryHeader
                }

                ListView {
                    id: libraryList
                    objectName: "libraryList"

                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    clip: true
                    spacing: 0
                    model: page.entries
                    currentIndex: page.entries.length > 0 ? 0 : -1
                    boundsBehavior: Flickable.StopAtBounds
                    ScrollBar.vertical: KScrollBar {}
                    focus: true
                    highlightFollowsCurrentItem: false
                    Keys.onPressed: function (event) {
                        if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
                            page.activateCurrent()
                            event.accepted = true
                        }
                    }

                    delegate: LibraryRow {
                        id: row
                        width: ListView.view.width - metrics.scrollBarSlot
                        entry: modelData
                        backend: page.backend
                        rowIndex: index
                        highlighted: ListView.isCurrentItem
                        thumbnailSlotWidth: density.thumbnailSlotWidth
                        thumbnailSlotHeight: density.thumbnailSlotHeight
                        primaryColumnWidth: density.sizeColumnWidth
                        secondaryColumnWidth: density.modifiedColumnWidth
                        columnSpacing: density.columnSpacing
                        contentPadding: density.rowPadding
                        rowHeight: density.rowHeight
                        textPixelSize: density.namePixelSize
                        metaPixelSize: density.metaPixelSize
                        iconPixelSize: density.iconPixelSize
                        videoBadgeSize: density.listVideoBadgeSize
                        videoBadgeMargin: density.listVideoBadgeMargin
                        playLoading: page.isPlayLoading(entry.id)
                        playing: page.isPlaying(entry.id)

                        onActivateRequested: function (id) {
                            if (page.backend)
                                page.backend.openLibraryEntry(id)
                        }
                        onPlayRequested: function (id, point) {
                            page.playEntry(id, row.mapToItem(null, point.x, point.y))
                        }
                    }
                }
            }

            BusyIndicator {
                anchors.centerIn: parent
                visible: page.loading
                running: visible
                width: 28
                height: 28
            }

            Label {
                objectName: "libraryEmptyHint"
                anchors.centerIn: parent
                width: Math.min(parent.width - 40, 420)
                visible: page.loaded && !page.loading && page.entries.length === 0
                         && page.errorText.length === 0
                text: page.emptyText
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.Wrap
                color: theme.textMuted
                font.pixelSize: theme.fontBody
            }

            Label {
                anchors.centerIn: parent
                width: Math.min(parent.width - 40, 420)
                visible: page.activeCollection && !page.loaded && !page.loading
                         && page.errorText.length > 0
                text: qsTr("资料库读取失败\n请稍后重试")
                horizontalAlignment: Text.AlignHCenter
                color: theme.textMuted
                font.pixelSize: theme.fontBody
            }
        }
    }

    FilesPagePlayMenu {
        id: playMenu
        backend: page.backend
    }

    Connections {
        target: page.backend

        function onChanged() {
            if (!page.backend || page.backend.libraryKind !== page.kind)
                return
            if (page.lastRevision === page.backend.libraryRevision)
                return
            page.lastRevision = page.backend.libraryRevision
            libraryList.currentIndex = page.entries.length > 0 ? 0 : -1
            if (page.visible)
                Qt.callLater(function () { libraryList.forceActiveFocus() })
        }
    }
}
