import QtQuick
import QtTest
import ".."

// 底栏容量指示：首次加载画骨架、到达后原地替换；不限量时保留文字、不画容量条。
TestCase {
    name: "QuotaIndicator"
    when: windowShown
    width: 400
    height: 40

    Component {
        id: quotaComponent

        QuotaIndicator {
            width: 300
            height: 32
        }
    }

    property var quota: null

    function init() {
        quota = quotaComponent.createObject(parent, { visible: true })
        verify(quota !== null, "QuotaIndicator 创建失败")
        quota.used = "1 GB"
        quota.limit = "10 GB"
        quota.loading = false
        quota.ratio = 0.42
        wait(0)
    }

    function cleanup() {
        if (quota)
            quota.destroy()
        quota = null
    }

    function test_shows_text_and_fill_when_ready() {
        const text = findChild(quota, "quotaText")
        const track = findChild(quota, "quotaTrack")
        const fill = findChild(quota, "quotaFill")
        const skeleton = findChild(quota, "quotaSkeleton")
        verify(text !== null && track !== null && fill !== null && skeleton !== null)

        compare(text.visible, true)
        compare(text.text, qsTr("已用 1 GB / 10 GB"))
        compare(track.visible, true)
        compare(Math.abs(fill.width - track.width * 0.42) < 0.5, true)
        compare(skeleton.visible, false)
    }

    function test_initial_load_shows_skeleton() {
        quota.used = ""
        quota.limit = ""
        quota.ratio = -1
        quota.loading = true
        wait(0)

        compare(findChild(quota, "quotaSkeleton").visible, true)
        compare(findChild(quota, "quotaSkeletonText").visible, true)
        compare(findChild(quota, "quotaSkeletonTrack").visible, true)
        compare(findChild(quota, "quotaText").visible, false)
        compare(findChild(quota, "quotaTrack").visible, false)

        quota.used = "1 GB"
        quota.limit = "10 GB"
        quota.ratio = 0.42
        quota.loading = false
        wait(0)

        compare(findChild(quota, "quotaSkeleton").visible, false)
        compare(findChild(quota, "quotaText").visible, true)
        compare(findChild(quota, "quotaTrack").visible, true)
    }

    function test_refresh_with_existing_values_keeps_them() {
        quota.loading = true
        wait(0)

        compare(findChild(quota, "quotaSkeleton").visible, false)
        compare(findChild(quota, "quotaText").visible, true)
        compare(findChild(quota, "quotaTrack").visible, true)
    }

    function test_unlimited_keeps_text_but_hides_bar() {
        quota.ratio = -1
        wait(0)

        compare(findChild(quota, "quotaText").visible, true)
        compare(findChild(quota, "quotaTrack").visible, false)
    }

    function test_missing_limit_hides_everything() {
        quota.limit = ""
        wait(0)

        compare(findChild(quota, "quotaText").visible, false)
        compare(findChild(quota, "quotaTrack").visible, false)
    }

    function test_fill_never_exceeds_track() {
        quota.ratio = 1.5
        wait(0)

        const track = findChild(quota, "quotaTrack")
        const fill = findChild(quota, "quotaFill")
        compare(Math.abs(fill.width - track.width) < 0.5, true)
    }
}
