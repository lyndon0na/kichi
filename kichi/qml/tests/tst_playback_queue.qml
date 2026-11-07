import QtQuick
import QtTest
import ".."

// 连播队列浮层：只读 backend.playQueue，点一行把队列下标发回去。
TestCase {
    name: "PlaybackQueue"
    when: windowShown
    width: 800
    height: 600

    QtObject {
        id: backend

        property var playQueue: [
            { "id": "a", "name": "[Group] Show [01][1080P].mkv", "episode": "01" },
            { "id": "b", "name": "[Group] Show [02][1080P].mkv", "episode": "" },
            { "id": "c", "name": "[Group] Show [10][1080P].mkv", "episode": "10" }
        ]
        property int playQueueIndex: 1
        property int clickedIndex: -1

        function playQueueItem(index) {
            clickedIndex = index
        }
    }

    PlaybackQueuePanel {
        id: panel
        backend: backend
        // 播放窗口会按窗口高度覆盖这个属性：必须保持可写。
        maxPopupHeight: 300
        visible: true
    }

    function test_rows_come_from_the_backend_queue() {
        wait(80)
        compare(panel.contentItem.count, 3, "队列有几集就显示几行")
        compare(panel.contentItem.itemAtIndex(1).selected, true, "当前集高亮")
    }

    function test_badge_shows_episode_or_queue_position() {
        wait(80)
        compare(panel.contentItem.itemAtIndex(0).badge, "01", "有集数就显示集数")
        compare(panel.contentItem.itemAtIndex(1).badge, "#2",
                "解析不出集数时明确标成队列位置")
        compare(panel.contentItem.itemAtIndex(2).badge, "10")
        compare(panel.contentItem.itemAtIndex(0).showPosition, true,
                "每一行都显示队列位置")
        compare(panel.contentItem.itemAtIndex(1).showPosition, true)
        compare(panel.contentItem.itemAtIndex(2).showPosition, true)
        compare(panel.contentItem.itemAtIndex(2).position, 3)
    }

    function test_clicking_a_row_jumps_to_that_index() {
        wait(80)
        panel.contentItem.itemAtIndex(2).clicked()
        compare(backend.clickedIndex, 2, "点第 3 行应跳到队列下标 2")
    }
}
