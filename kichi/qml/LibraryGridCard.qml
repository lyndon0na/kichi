import QtQuick

// 资料库网格卡片：标准网格和播放历史分区网格共用同一套接线。
FileGridItem {
    id: card

    property var libraryPage: null
    property alias entryData: card.entry
    property int entryIndex: -1
    property var density: null

    signal entrySelected(int entryIndex, int modifiers)
    signal entryFolderOpenRequested(string id)
    signal entryPlayRequested(string id, point position)
    signal entryContextMenuRequested(var entry, point position)

    rowIndex: entryIndex
    backend: libraryPage ? libraryPage.backend : null
    downloadNonPlayable: false
    selected: !!entryData && libraryPage
              && String(libraryPage.currentEntryId) === String(entryData.id)
    contentPadding: density ? density.gridCardPadding : 5
    cardSpacing: density ? density.gridCardSpacing : 7
    nameLineHeight: density ? density.gridNameLineHeight : 19
    nameBlockHeight: density ? density.gridNameBlockHeight : 38
    thumbnailRadius: density ? density.gridThumbnailRadius(width) : 8
    thumbnailDecodeWidth: density ? density.gridThumbnailDecodeWidth : 640
    thumbnailDecodeHeight: density ? density.gridThumbnailDecodeHeight : 427
    textPixelSize: density ? density.namePixelSize : 14
    detailPixelSize: density ? density.metaPixelSize : 12
    detailLineHeight: density ? density.gridDetailLineHeight : 17
    iconPixelSize: density ? density.gridIconPixelSize : 28
    videoBadgeSize: density ? density.gridVideoBadgeSize : 28
    videoBadgeMargin: density ? density.gridVideoBadgeMargin : 7
    playLoading: libraryPage && entryData
                 ? libraryPage.isPlayLoading(entryData.id) : false
    playing: libraryPage && entryData ? libraryPage.isPlaying(entryData.id) : false
    detailText: libraryPage ? libraryPage.gridDetailText(entryData) : ""

    onSelectRequested: function (modifiers, itemIndex) {
        card.entrySelected(card.entryIndex, modifiers)
    }
    onFolderOpenRequested: function (id) {
        card.entryFolderOpenRequested(id)
    }
    onPlayRequested: function (id, point) {
        card.entryPlayRequested(id, card.mapToItem(null, point.x, point.y))
    }
    onContextMenu: function (point) {
        card.entryContextMenuRequested(
            card.entryData, card.mapToItem(null, point.x, point.y))
    }
}
