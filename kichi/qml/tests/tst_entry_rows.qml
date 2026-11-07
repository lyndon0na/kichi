import QtQuick
import QtTest
import "../files"

TestCase {
    name: "EntryRows"
    when: windowShown
    width: 320
    height: 220

    EntryRows {
        id: rows
    }

    ListView {
        id: list
        width: 300
        height: 180
        model: rows.model
        delegate: Item {
            required property var entry
            required property int index

            width: list.width
            height: 40
        }
    }

    function entry(id, name, dateGroup) {
        return {
            id: id,
            name: name,
            dateGroup: dateGroup || ""
        }
    }

    function init() {
        rows.source = []
        list.contentY = 0
        wait(0)
    }

    function test_sync_keeps_scroll_position_and_updates_in_place() {
        const initial = []
        for (let i = 0; i < 30; ++i)
            initial.push(entry("id-" + i, "name-" + i))
        rows.source = initial
        compare(rows.count, 30)
        tryVerify(function () { return list.contentHeight >= 1200 })
        list.contentY = 440

        const next = initial.slice()
        next[4] = entry("id-4", "renamed")
        next.push(entry("new", "new"))
        next.splice(0, 1)
        rows.source = next

        compare(rows.count, 30)
        compare(rows.model.get(3).entry.name, "renamed")
        tryCompare(list, "contentY", 440)
    }

    function test_date_group_role_tracks_entry_changes() {
        rows.source = [
            entry("a", "A", "today"),
            entry("b", "B", "yesterday")
        ]
        compare(rows.model.get(0).dateGroup, "today")
        compare(rows.model.get(1).dateGroup, "yesterday")

        rows.source = [
            entry("a", "A", "earlier"),
            entry("b", "B", "yesterday")
        ]
        compare(rows.model.get(0).dateGroup, "earlier")
    }
}
