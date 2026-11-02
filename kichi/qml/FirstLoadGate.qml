import QtQuick

// 每个数据页只在本次登录后的第一次读取显示骨架。
//
// `start()` 必须在页面首次变为 active、并已经发起 ensure / refresh 之后调用：
// 有请求就记下这次加载，没有请求（命中缓存）则直接结束首次状态。
// 后续 loading 再变化时，`pending` 已经为 false，不会重新显示骨架。
QtObject {
    id: gate

    /// 页面当前是否在前台 / 可见。
    property bool active: false
    /// 登录会话仍有效。退出登录时重置，下次登录重新允许一次骨架。
    property bool sessionActive: true
    /// 当前数据请求是否在途。
    property bool loading: false
    /// 当前页面的条目数。已经有数据时不覆盖旧内容。
    property int count: 0

    property bool pending: true
    property bool observedLoading: false

    readonly property bool showSkeleton: gate.pending && gate.active
                                         && gate.observedLoading && gate.loading
                                         && gate.count === 0

    onSessionActiveChanged: {
        if (!gate.sessionActive)
            gate.reset()
    }

    onLoadingChanged: gate.note()

    function start() {
        if (!gate.active || !gate.pending || !gate.sessionActive)
            return
        if (gate.loading) {
            gate.observedLoading = true
            return
        }
        gate.consume()
    }

    function note() {
        if (!gate.pending)
            return
        if (gate.loading) {
            if (gate.active)
                gate.observedLoading = true
            return
        }
        if (gate.observedLoading)
            gate.consume()
    }

    function consume() {
        gate.pending = false
        gate.observedLoading = false
    }

    function reset() {
        gate.pending = true
        gate.observedLoading = false
    }
}
