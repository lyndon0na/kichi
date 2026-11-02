import QtQuick
import QtTest
import org.kde.kirigami as Kirigami
import ".."

// 动效时长必须走 Kirigami 的令牌，这样 KDE 的全局动画速度设置才会生效。
TestCase {
    name: "ThemeMotion"

    KTheme { id: theme }

    function test_motion_tokens_follow_kirigami_units() {
        compare(theme.motionFast, Kirigami.Units.shortDuration)
        compare(theme.motionNormal, Kirigami.Units.longDuration)
        compare(theme.motionPageLift, 3)
    }
}
