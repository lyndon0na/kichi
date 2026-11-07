import QtQuick

// 数据页内容骨架的共享判定。
//
// 每次页面在前台发起真实读取、且当前没有可显示条目时都显示骨架。
// 页面把“不属于本次请求的旧内容”计为 0；属于当前页面的旧内容则保留，
// 刷新结束前不闪空。状态完全由绑定派生，页面不需要再手动 start / consume。
QtObject {
    id: gate

    /// 页面当前是否在前台 / 可见。
    property bool active: false
    /// 登录会话仍有效；退出登录期间不画内容骨架。
    property bool sessionActive: true
    /// 当前数据请求是否在途。
    property bool loading: false
    /// 属于本次请求、可直接显示的条目数。已经有数据时不覆盖旧内容。
    property int count: 0

    readonly property bool showSkeleton: gate.active && gate.sessionActive
                                         && gate.loading && gate.count === 0
}
