import QtQuick
import QtTest
import "../library"

TestCase {
    name: "LibrarySections"
    when: windowShown
    width: 760
    height: 520

    Component {
        id: viewComponent

        LibrarySectionView {
            width: 760
            height: 500
            headerHeight: 38
            itemRowHeight: 200
            delegate: Item {
                required property int sourceIndex
                required property real rowOffset
                required property real rowExtent
                y: rowOffset
                height: rowExtent
                width: 750
            }
        }
    }

    property var view: null

    function init() {
        view = createTemporaryObject(viewComponent, parent)
        verify(view !== null)
    }

    function layoutRows(counts) {
        const result = []
        for (let group = 0; group < counts.length; ++group) {
            if (counts[group] === 0)
                continue
            result.push({ key: "header-" + group, rowType: "header" })
            for (let row = 0; row < counts[group]; ++row)
                result.push({ key: group + "-" + row, rowType: "items" })
        }
        return result
    }

    function verifyViewport(expectedHeight) {
        compare(view.contentHeight, expectedHeight)
        compare(view.originY, 0)
        const middle = view.rowIndexAt(view.contentY + Math.min(
                                          view.height / 2,
                                          expectedHeight - view.contentY - 1))
        if (middle >= 0) {
            const item = view.itemAtRow(middle)
            verify(item !== null, "视口中的行必须已经实例化")
            compare(item.y, view.offsets[middle])
        }
        verify(view.instantiatedRows <= Math.ceil(view.height / view.itemRowHeight) + 7,
               "长历史只创建视口附近的行")
    }

    function test_scroll_height_and_origin_stay_exact_data() {
        return [
            { tag: "grid-66", counts: [1, 2, 15], extent: 200 },
            { tag: "list-66", counts: [2, 5, 59], extent: 48 },
            { tag: "one-group", counts: [0, 0, 17], extent: 200 }
        ]
    }

    function test_scroll_height_and_origin_stay_exact(data) {
        view.itemRowHeight = data.extent
        view.rows = layoutRows(data.counts)
        view.forceLayout()
        const expected = data.counts.reduce(function (sum, count) {
            return sum + (count > 0 ? 38 + count * data.extent : 0)
        }, 0)
        const ratio = view.visibleArea.heightRatio
        const bottom = Math.max(0, expected - view.height)
        for (let y = 0; y <= bottom + 60; y += 60) {
            view.contentY = Math.min(y, bottom)
            wait(1)
            verifyViewport(expected)
            compare(view.visibleArea.heightRatio, ratio)
        }
        const last = view.itemAtRow(view.rows.length - 1)
        verify(last !== null)
        compare(last.y + last.height, expected)
    }

    function test_jump_resize_and_refresh_keep_positions_with_2000_rows() {
        view.rows = layoutRows([2, 5, 1993])
        view.forceLayout()
        const total = 3 * 38 + 2000 * 200

        // 滚动条从顶部直接拖到底部，再跳回顶部和分区边界。
        view.contentY = total - view.height
        wait(1)
        verifyViewport(total)
        compare(view.itemAtRow(2002).y + view.itemAtRow(2002).height, total)
        view.contentY = 0
        wait(1)
        compare(view.itemAtRow(0).y, 0)
        view.positionViewAtRow(10, ListView.Beginning)
        compare(view.contentY, 3 * 38 + 7 * 200)

        const oldY = view.contentY
        view.rows = layoutRows([2, 5, 1994])
        view.forceLayout()
        compare(view.contentY, oldY, "后台新增记录不能重置滚动位置")
        view.itemRowHeight = 160
        view.height = 380
        view.forceLayout()
        verifyViewport(3 * 38 + 2001 * 160)
        view.positionViewAtRow(2003, ListView.End)
        const last = view.itemAtRow(2003)
        compare(last.y + last.height - view.contentY, view.height)

        view.rows = layoutRows([1, 0, 0])
        view.forceLayout()
        compare(view.contentY, 0)
        compare(view.contentHeight, 38 + 160)
        compare(view.instantiatedRows, 2)
        view.rows = []
        view.forceLayout()
        compare(view.contentHeight, 0)
        compare(view.instantiatedRows, 0)
    }
}
