import QtQuick
import QtTest
import ".."

// 账号区视觉回归：昵称优先、账号兜底、头像 / 会员标保留系统强调色。
TestCase {
    name: "AccountCard"
    when: windowShown
    width: 280
    height: 140

    AccountCard {
        id: card
        width: 240
    }

    function findNamed(item, name) {
        if (!item)
            return null
        if (item.objectName === name)
            return item
        var children = item.children || []
        for (var i = 0; i < children.length; i++) {
            var found = findNamed(children[i], name)
            if (found)
                return found
        }
        return null
    }

    function init() {
        card.account = "mail@example.com"
        card.displayName = "夏日用户"
        card.avatarPath = ""
        card.quotaUsed = "1 GB"
        card.quotaLimit = "10 GB"
        card.quotaLoading = false
        card.quotaRatio = 0.1
        card.vip = true
    }

    function test_nickname_takes_priority_and_account_is_fallback() {
        compare(card.shownName, "夏日用户")
        compare(card.identityPending, false)
        card.displayName = ""
        wait(0)
        compare(card.shownName, "mail")
    }

    function test_avatar_and_member_badge_use_accent_colors() {
        wait(0)
        var avatar = findNamed(card, "avatarCircle")
        var badge = findNamed(card, "memberBadge")
        var initial = findNamed(card, "avatarInitial")
        verify(avatar !== null && badge !== null && initial !== null)
        compare(String(avatar.color), String(card.theme.accentGhost))
        compare(String(initial.color), String(card.theme.accent))
        compare(String(badge.color), String(card.theme.accentGhost))
        verify(card.vip)
    }

    function test_identity_pending_only_when_name_and_account_are_both_missing() {
        card.displayName = ""
        card.account = ""
        wait(0)
        compare(card.identityPending, true)
        card.account = "fallback@example.com"
        wait(0)
        compare(card.identityPending, false)
        compare(card.shownName, "fallback")
    }

    function test_quota_skeleton_and_bar_reserve_height() {
        card.displayName = "夏日用户"
        card.quotaUsed = ""
        card.quotaLimit = ""
        card.quotaLoading = true
        card.quotaRatio = -1
        wait(0)

        var pendingHeight = card.implicitHeight
        var slot = findNamed(card, "quotaTextSlot")
        var skeleton = findNamed(card, "quotaSkeleton")
        var bar = findNamed(card, "quotaBar")
        verify(slot !== null && skeleton !== null && bar !== null)
        compare(card.identityPending, false)
        compare(card.quotaPending, true)
        compare(skeleton.showing, true)
        compare(slot.height, 16)
        compare(bar.height, 3)

        card.quotaLoading = false
        card.quotaUsed = "1 GB"
        card.quotaLimit = "10 GB"
        card.quotaRatio = 0.1
        wait(0)

        compare(card.quotaPending, false)
        compare(skeleton.showing, false)
        compare(slot.height, 16)
        compare(bar.height, 3)
        compare(card.implicitHeight, pendingHeight)
    }
}
