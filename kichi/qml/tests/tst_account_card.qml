import QtQuick
import QtTest
import "../shell"

// 账号区视觉回归：昵称优先、账号兜底（悬停提示展示邮箱）、头像保留系统强调色、设置入口在行尾。
TestCase {
    name: "AccountCard"
    when: windowShown
    width: 280
    height: 140

    AccountCard {
        id: card
        width: 240
    }

    Component {
        id: accountCardComponent

        AccountCard {
            width: 240
        }
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
        card.quotaUsed = ""
        card.quotaLimit = ""
        card.quotaLoading = false
        card.quotaRatio = -1
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

    function test_single_line_keeps_height_and_hides_email() {
        // 声明式控件在 Qt Test 里默认隐藏，尺寸断言用动态创建的一份
        const testCard = accountCardComponent.createObject(parent, { visible: true })
        verify(testCard !== null, "AccountCard 创建失败")
        testCard.account = "mail@example.com"
        testCard.displayName = "夏日用户"
        wait(0)

        compare(findNamed(testCard, "accountEmail"), null, "账号区不再显示邮箱行")
        compare(findNamed(testCard, "avatarCircle").width, 28)

        // 加载骨架和真实文字同一行高：身份数据到达时账号区不抽动
        const loadedHeight = testCard.implicitHeight
        testCard.displayName = ""
        testCard.account = ""
        wait(0)
        compare(testCard.identityPending, true)
        tryCompare(testCard, "implicitHeight", loadedHeight, 1000)

        testCard.destroy()
    }

    function test_quota_line_fills_account_area() {
        const testCard = accountCardComponent.createObject(parent, { visible: true })
        verify(testCard !== null, "AccountCard 创建失败")
        testCard.account = "mail@example.com"
        testCard.displayName = "夏日用户"
        testCard.quotaUsed = "4.7 TB"
        testCard.quotaLimit = "10.0 TB"
        testCard.quotaRatio = 0.47
        wait(0)

        const text = findNamed(testCard, "quotaText")
        const track = findNamed(testCard, "quotaTrack")
        verify(text !== null && track !== null, "账号区缺少容量指示")
        compare(text.visible, true)
        compare(text.text, qsTr("已用 4.7 TB / 10.0 TB"))
        compare(track.visible, true)

        // 容量条铺满账号区内容宽度，位于名字行下方
        const settingsButton = findNamed(testCard, "accountSettingsButton")
        tryCompare(track, "width", testCard.width - 2 * testCard.theme.gapMd, 1000)
        verify(track.mapToItem(testCard, 0, 0).y
               > settingsButton.mapToItem(testCard, 0, 0).y,
               "容量条应在账号行下方")

        // 上限未知且不在加载：整块收起，不占账号区高度
        testCard.quotaUsed = ""
        testCard.quotaLimit = ""
        testCard.quotaRatio = -1
        wait(0)
        compare(findNamed(testCard, "accountQuota").visible, false)

        testCard.destroy()
    }

}
