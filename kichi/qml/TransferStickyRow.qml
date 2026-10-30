import QtQuick

// 文件夹子行滚动时，在列表顶部叠一层与父行同款的吸附行。
//
// 不把父行从 ListModel 里搬走：原父行仍占自己的位置，离开子文件范围后
// 这层自然隐藏。数据仍由页面传入，进度通过 transferLive 单独刷新。
Item {
    id: sticky

    property var rowData: null
    property var live: ({})
    property var wheelTarget: null
    property bool showSize: true
    property bool showRate: true
    property bool flash: false

    signal toggleRequested(string key)
    signal cancelRequested(string key)
    signal retryRequested(string key)
    signal revealRequested(string key)

    objectName: "transferStickyRow"
    implicitHeight: density.rowHeight
    height: implicitHeight

    function value(field, fallback) {
        if (sticky.rowData === null || sticky.rowData === undefined)
            return fallback
        var result = sticky.rowData[field]
        return result === undefined || result === null ? fallback : result
    }

    TransferTableDensity { id: density }

    // 吸附层盖在 ListView 上方，滚轮落在父行上时仍交给下面的列表。
    WheelHandler {
        target: sticky.wheelTarget
        property: "contentY"
        orientation: Qt.Vertical
    }

    TransferMainRow {
        anchors.fill: parent
        key: sticky.value("key", "")
        name: sticky.value("name", "")
        sourceId: sticky.value("sourceId", "")
        phase: sticky.value("phase", "")
        stateText: sticky.value("stateText", "")
        running: sticky.value("running", false)
        progress: sticky.value("progress", 0)
        progressText: sticky.value("progressText", "")
        sizeText: sticky.value("sizeText", "")
        detail: sticky.value("detail", "")
        path: sticky.value("path", "")
        folder: sticky.value("folder", true)
        childCount: sticky.value("childCount", 0)
        failedCount: sticky.value("failedCount", 0)
        depth: 0
        rowKind: "folder"
        parentKey: ""
        expandable: sticky.value("expandable", true)
        expanded: true
        sticky: true
        live: sticky.live
        showSize: sticky.showSize
        showRate: sticky.showRate
        flash: sticky.flash
        onToggleRequested: function(key) { sticky.toggleRequested(key) }
        onCancelRequested: function(key) { sticky.cancelRequested(key) }
        onRetryRequested: function(key) { sticky.retryRequested(key) }
        onRevealRequested: function(key) { sticky.revealRequested(key) }
    }
}
