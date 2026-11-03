import QtQuick
import QtQuick.Controls
import org.kde.kirigami as Kirigami

// 全应用视觉令牌（圆角 / 间距 / 字号 / 表面色）。
//
// 原则没有变：颜色仍然全部从 `palette.*` 推导，明暗跟随 Plasma 桌面设置，
// 不自带两套色板（见 docs/ui.md）。这里只是把以前散在各处的
// 「12 / 4 / 0.05」收成有名字的常量，改一处全应用一起变。
//
// 用法：在需要的地方实例化一个 `KTheme { id: theme }`，读 `theme.radiusCard`
// 之类的值。它本身不可见、尺寸为 0，不参与布局。
//
// ⚠ 字号只走三档 + 两个辅助档：标题 18 / 小标题 14 / 正文 13 / 次要 12 / 说明 11。
//   不要再随手加第五、第六档（docs/ui.md「排版」一节）。
Item {
    id: theme

    visible: false
    width: 0
    height: 0

    /// 当前是深色主题（用来给状态色选深/浅变体）
    readonly property bool darkMode: palette.base.hsvValue < 0.5

    // —— 圆角 ——
    // 跟 KDE 控件同一档：Breeze 用 Kirigami.Units.cornerRadius = 5，
    // 输入框 / 按钮 / 弹窗都是这个值。再往上是我们的卡片与容器，
    // 各多 1~3px，但整体仍然收在「桌面应用」的克制尺度里。
    /// 小控件（图标按钮 / 药丸标签）
    readonly property int radiusSmall: 4
    /// 输入框 / 普通按钮（= KDE 控件圆角）
    readonly property int radiusControl: 5
    /// 列表卡片 / 侧栏选中项
    readonly property int radiusCard: 6
    /// 弹窗 / 登录卡片这类大容器
    readonly property int radiusSurface: 8

    // —— 间距 ——
    readonly property int gapXs: 4
    readonly property int gapSm: 8
    readonly property int gapMd: 12
    readonly property int gapLg: 16
    readonly property int gapXl: 24

    // —— 字号 ——
    readonly property int fontTitle: 18
    readonly property int fontSection: 14
    readonly property int fontBody: 13
    readonly property int fontMeta: 12
    readonly property int fontCaption: 11

    // —— 动效 ——
    // 时长直接取 Kirigami 的令牌：org.kde.desktop 平台插件会把 KDE
    // 「动画速度」（AnimationDurationFactor）乘进去；关闭动画时归零或接近零。
    /// 快速状态反馈：侧栏选中
    readonly property int motionFast: Kirigami.Units.shortDuration
    /// 页面级过渡与侧栏竖条滑动
    readonly property int motionNormal: Kirigami.Units.longDuration
    /// 页面标题淡入时的轻微上移距离
    readonly property int motionPageLift: 3

    // —— 强调色（= Plasma 系统强调色，Primary 按钮 / 选中态 / 关键高亮） ——
    readonly property color accent: palette.highlight
    // ⚠ 不要命名为 onAccent / onDanger：QML 会把 on* 解析成信号处理器，
    // 这两个属性会静默回落成无效颜色（文字变黑），见 docs/pitfalls.md 第 55 条。
    readonly property color accentForeground: palette.highlightedText
    /// 选中态底色：一层很淡的强调色，不铺满饱和色块
    readonly property color accentSoft: Qt.rgba(accent.r, accent.g, accent.b,
                                                theme.darkMode ? 0.22 : 0.12)
    /// 悬停 / 徽标底：比 accentSoft 更淡
    readonly property color accentGhost: Qt.rgba(accent.r, accent.g, accent.b,
                                                 theme.darkMode ? 0.14 : 0.08)
    /// 完全透明、但 RGB 与 accentGhost 相同的「消失色」。
    readonly property color accentClear: Qt.rgba(accentGhost.r, accentGhost.g,
                                                 accentGhost.b, 0)

    // —— 文字 ——
    readonly property color textPrimary: palette.text
    readonly property color textSecondary: Qt.rgba(palette.text.r, palette.text.g,
                                                   palette.text.b, 0.72)
    readonly property color textMuted: palette.placeholderText

    // —— 表面 ——
    /// 内容底色（窗口底色）
    readonly property color surface: palette.base
    /// 抬升一档的表面：卡片 / 输入框 / 次要按钮
    /// 用正文色加透明度而不是写死 #F9FAFB：深色主题下会自动变成「比底色亮一点」
    readonly property color surfaceRaised: Qt.tint(palette.base,
        Qt.rgba(palette.text.r, palette.text.g, palette.text.b,
                theme.darkMode ? 0.07 : 0.035))
    readonly property color surfaceHover: Qt.tint(palette.base,
        Qt.rgba(palette.text.r, palette.text.g, palette.text.b,
                theme.darkMode ? 0.11 : 0.05))
    readonly property color surfacePressed: Qt.tint(palette.base,
        Qt.rgba(palette.text.r, palette.text.g, palette.text.b,
                theme.darkMode ? 0.16 : 0.09))
    /// 完全透明、但 RGB 与 `surfaceHover` 相同的「消失色」。
    /// ⚠ 悬停淡入淡出**不要**写 `"transparent"`：隐式的透明色是 rgba(0,0,0,0)，
    /// ColorAnimation 会按 RGBA 分量线性插值，半程变成「黑色 50% 透明」，
    /// 在浅色底上看起来就是悬停/移出各闪一下灰黑（2026-10-29 用户实测）。
    /// 用这个颜色插值，RGB 始终不变、只有 alpha 在动，才是干净的淡入淡出。
    readonly property color surfaceClear: Qt.rgba(surfaceHover.r, surfaceHover.g,
                                                  surfaceHover.b, 0)
    /// 弹窗表面：比内容底色略深/略浅一档，和卡片区分
    readonly property color surfaceSheet: palette.window

    // —— 线 ——
    /// 极浅的分隔线（浅色下接近 #E5E7EB），取代原来偏硬的 palette.mid
    readonly property color hairline: Qt.rgba(palette.text.r, palette.text.g,
                                              palette.text.b,
                                              theme.darkMode ? 0.18 : 0.10)
    /// 更淡的装饰线（页头 / 卡片内分隔）
    readonly property color divider: Qt.rgba(palette.text.r, palette.text.g,
                                             palette.text.b,
                                             theme.darkMode ? 0.12 : 0.06)

    /// 视频缩略图上的深色药丸 / 遮罩：不跟内容底色绑定
    readonly property color mediaBadge: Qt.rgba(palette.shadow.r, palette.shadow.g,
                                                palette.shadow.b,
                                                0.82)
    /// 缩略图上的轻量角标底色：仍能压住复杂画面，但不会像实心黑块。
    readonly property color mediaBadgeSoft: Qt.rgba(palette.shadow.r,
                                                    palette.shadow.g,
                                                    palette.shadow.b,
                                                    0.44)
    /// 加载态覆盖缩略图的暗幕；小缩略图再深一档，保证白色加载环清楚。
    readonly property color mediaScrim: Qt.rgba(palette.shadow.r,
                                                palette.shadow.g,
                                                palette.shadow.b,
                                                0.38)
    readonly property color mediaScrimStrong: Qt.rgba(palette.shadow.r,
                                                      palette.shadow.g,
                                                      palette.shadow.b,
                                                      0.68)
    /// 深色媒体遮罩上的高对比前景。
    /// 不直接使用 `palette.brightText`：KDE 浅色主题下它不保证是浅色。
    /// 从当前明暗方案的文字 / 底色中取亮的一侧，仍随 Plasma 配色实时变化。
    readonly property color mediaForeground: theme.darkMode ? palette.text
                                                            : palette.base

    // —— 状态色（全应用仅存的字面量颜色，深色主题用浅一档的变体保证对比度） ——
    readonly property color danger: theme.darkMode ? "#f08c84" : "#c0392b"
    readonly property color dangerForeground: theme.darkMode ? "#221512" : "#ffffff"
    readonly property color warning: theme.darkMode ? "#e0b15a" : "#b9770e"
    readonly property color success: theme.darkMode ? "#7fd39a" : "#3c9d5a"

    /// 某状态色对应的淡底（徽标 / 提示用）
    function softTint(color, alpha) {
        return Qt.rgba(color.r, color.g, color.b, alpha === undefined ? 0.12 : alpha)
    }
}
