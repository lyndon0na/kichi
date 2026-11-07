import QtQuick
import QtQuick.Controls
import "../components"

// 日期分区的行高已知：按完整布局定位，滚动时只维护视口附近的委托。
Flickable {
    id: view

    property var rows: []
    property real itemRowHeight: 48
    property real headerHeight: 38
    property Component delegate: null
    readonly property int instantiatedRows: activeRows.count
    readonly property var offsets: {
        const result = [0]
        for (let i = 0; i < rows.length; ++i) {
            const extent = rows[i].rowType === "header" ? headerHeight : itemRowHeight
            result.push(result[i] + extent)
        }
        return result
    }
    property int firstRow: -1
    property int lastRow: -1

    signal keyPressed(var event)

    contentWidth: width
    contentHeight: offsets[offsets.length - 1]
    flickableDirection: Flickable.VerticalFlick
    boundsBehavior: Flickable.StopAtBounds
    clip: true
    focus: true
    ScrollBar.vertical: KScrollBar {}

    ListModel { id: activeRows }

    Repeater {
        id: rowRepeater
        model: activeRows
        delegate: view.delegate
    }

    function rowIndexAt(y) {
        if (y < 0 || y >= contentHeight || rows.length === 0)
            return -1
        let low = 0
        let high = rows.length
        while (low < high) {
            const middle = Math.floor((low + high) / 2)
            if (offsets[middle + 1] <= y)
                low = middle + 1
            else
                high = middle
        }
        return low
    }

    function itemAtRow(rowIndex) {
        if (rowIndex < firstRow || rowIndex > lastRow)
            return null
        return rowRepeater.itemAt(rowIndex - firstRow)
    }

    function updateRows(force) {
        if (!visible)
            return
        const buffer = Math.max(itemRowHeight, headerHeight)
        const first = rowIndexAt(Math.max(0, contentY - buffer))
        const last = rowIndexAt(Math.min(contentHeight - 1,
                                        contentY + height + buffer))
        if (!force && first === firstRow && last === lastRow)
            return
        firstRow = first
        lastRow = last

        const wanted = ({})
        for (let i = first; i >= 0 && i <= last; ++i)
            wanted[String(rows[i].key)] = i
        for (let stale = activeRows.count - 1; stale >= 0; --stale) {
            if (wanted[String(activeRows.get(stale).key)] === undefined)
                activeRows.remove(stale)
        }
        for (let sourceIndex = first; sourceIndex >= 0 && sourceIndex <= last; ++sourceIndex) {
            const target = sourceIndex - first
            const key = String(rows[sourceIndex].key)
            let found = -1
            for (let scan = target; scan < activeRows.count; ++scan) {
                if (String(activeRows.get(scan).key) === key) {
                    found = scan
                    break
                }
            }
            if (found < 0) {
                activeRows.insert(target, {
                    key: key,
                    sourceIndex: sourceIndex,
                    rowData: rows[sourceIndex],
                    rowOffset: offsets[sourceIndex],
                    rowExtent: offsets[sourceIndex + 1] - offsets[sourceIndex]
                })
            } else {
                if (found !== target)
                    activeRows.move(found, target, 1)
                const current = activeRows.get(target)
                // 保留可见行的快照，刷新删行时不让旧委托读到失效下标。
                if (force)
                    activeRows.set(target, { rowData: rows[sourceIndex] })
                if (current.sourceIndex !== sourceIndex)
                    activeRows.setProperty(target, "sourceIndex", sourceIndex)
                if (current.rowOffset !== offsets[sourceIndex])
                    activeRows.setProperty(target, "rowOffset", offsets[sourceIndex])
                const extent = offsets[sourceIndex + 1] - offsets[sourceIndex]
                if (current.rowExtent !== extent)
                    activeRows.setProperty(target, "rowExtent", extent)
            }
        }
    }

    function forceLayout() {
        if (!visible)
            return
        contentY = Math.max(0, Math.min(contentY, contentHeight - height))
        updateRows(true)
    }

    function positionViewAtRow(rowIndex, mode) {
        if (rowIndex < 0 || rowIndex >= rows.length)
            return
        cancelFlick()
        const top = offsets[rowIndex]
        const bottom = offsets[rowIndex + 1]
        let next = contentY
        if (mode === ListView.Beginning)
            next = top
        else if (mode === ListView.End)
            next = bottom - height
        else if (mode === ListView.Center)
            next = (top + bottom - height) / 2
        else if (top < contentY)
            next = top
        else if (bottom > contentY + height)
            next = bottom - height
        contentY = Math.max(0, Math.min(next, contentHeight - height))
        updateRows(false)
    }

    onContentYChanged: updateRows(false)
    onOffsetsChanged: Qt.callLater(view.forceLayout)
    onRowsChanged: {
        if (!rows || rows.length === 0) {
            activeRows.clear()
            firstRow = -1
            lastRow = -1
        }
    }
    onHeightChanged: Qt.callLater(view.forceLayout)
    onVisibleChanged: {
        if (visible)
            forceLayout()
    }
    Component.onCompleted: forceLayout()

    Keys.onPressed: function (event) {
        view.keyPressed(event)
    }
}
