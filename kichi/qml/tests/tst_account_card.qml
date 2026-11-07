import QtQuick
import QtTest
import "../shell"

// 账号区视觉回归：昵称优先、账号兜底、头像保留系统强调色、设置入口在行尾。
TestCase {
    name: "AccountCard"
    when: windowShown
    width: 280
    height: 140

    AccountCard {
        id: card
        width: 240
    }

    SignalSpy {
        id: settingsSpy
        target: card
        signalName: "settingsRequested"
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
        card.settingsActive = false
    }

    function test_nickname_takes_priority_and_account_is_fallback() {
        compare(card.shownName, "夏日用户")
        compare(card.identityPending, false)
        card.displayName = ""
        wait(0)
        compare(card.shownName, "mail")
    }

    function test_avatar_uses_accent_colors() {
        wait(0)
        var avatar = findNamed(card, "avatarCircle")
        var initial = findNamed(card, "avatarInitial")
        verify(avatar !== null && initial !== null)
        compare(String(avatar.color), String(card.theme.accentGhost))
        compare(String(initial.color), String(card.theme.accent))
        compare(findNamed(card, "memberBadge"), null, "会员标已去掉")
    }

    function test_settings_button_follows_active_state() {
        var button = findNamed(card, "accountSettingsButton")
        verify(button !== null, "账号行缺少设置入口")
        compare(button.checked, false)

        card.settingsActive = true
        wait(0)
        compare(button.checked, true)

        settingsSpy.clear()
        button.clicked()
        compare(settingsSpy.count, 1)
    }

    function test_avatar_reload_keeps_mask_effect_alive() {
        const image = findChild(card, "avatarImage")
        verify(image !== null)
        card.avatarPath = Qt.resolvedUrl("../../assets/icons/hicolor/scalable/apps/kichi.svg")
        tryCompare(image, "status", Image.Ready)
        const effect = findChild(card, "avatarMaskEffect")
        verify(effect !== null)
        card.avatarPath = ""
        tryCompare(image, "status", Image.Null)
        compare(findChild(card, "avatarMaskEffect"), effect,
                "图片重载不能同步销毁 Qt 正在遍历的遮罩层")
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

}
