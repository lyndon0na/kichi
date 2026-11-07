pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 底部居中的播放加载 / 错误提示条。
//
// 只占自身尺寸，不铺满：`anchors.fill` 的透明层会吃掉文件列表的点击，
// `visible` 必须是真 false，不只是透明。
//
// 三个状态（优先级：错误 > 加载 > 回退提示）：
// - 加载：仅当加载中的条目不在当前列表里（切目录 / 退出搜索 / 换标签页 /
//   离开对应页面）时兜底显示，延迟 400 ms；
// - 错误：播放失败。立即出现，[关闭] 清掉；
// - 提示：档位回退说明（ADR 0003），3 秒后自己收起。
Rectangle {
    id: toast

    property var backend: null
    /// 当前列表正内联显示加载 / 取消；那里不要再叠一条底部提示。
    /// 加载条目已经不在列表里（切目录 / 退出搜索 / 换标签页）时必须为 false。
    property bool inlineLoading: false

    readonly property bool loading: toast.backend !== null
                                    && toast.backend.playLoading === true
                                    && !toast.inlineLoading
    readonly property bool failed: toast.backend !== null && toast.backend.playError !== ""
    readonly property bool notice: toast.backend !== null
                                   && toast.backend.playNotice !== ""
                                   && !toast.loading && !toast.failed

    /// 是否真的显示（内部定时器控制；测试也读它）
    property bool shown: false
    visible: shown

    KTheme { id: theme }

    // 浮在页面之上，但只占自己那一块
    z: 50
    radius: theme.radiusCard
    color: theme.surfaceRaised
    border.width: 1
    border.color: theme.hairline

    /// 内容四周留白（也是内部 RowLayout 的边距）
    property int padding: 12

    // 内容宽度上限：长文件名只把文字截断，不能把窗口撑变形
    readonly property int maxTextWidth: 420

    implicitWidth: layout.implicitWidth + 2 * toast.padding
    implicitHeight: layout.implicitHeight + 2 * toast.padding

    /// 提示条落在底栏上方。
    ///
    /// 实测：ApplicationWindow 的 `footer` 会把 `contentItem` 的高度扣掉，
    /// 所以贴着 `parent.bottom` 就已经在底栏之上，不需要（也不能）直接
    /// `anchors.bottom: footer.top` —— 底栏不是兄弟节点，那样写会触发
    /// 「cannot anchor to an item that isn't a parent or sibling」。
    anchors.horizontalCenter: parent.horizontalCenter
    anchors.bottom: parent.bottom
    anchors.bottomMargin: 12

    // 状态去重：同一条提示不因为别的 changed 信号重复弹出
    property string stateKey: ""

    /// 400 ms 延迟：取链很快时不闪一下
    ///
    /// `triggeredOnStart` 不要开：`stop()` 后再 `restart()` 的语义在
    /// Qt 6 里是重新计时，不会补一次立即触发。
    Timer {
        id: showTimer
        interval: 400
        onTriggered: toast.shown = true
    }

    /// 回退提示的停留时间。到时收起并让后端清掉那份文案，
    /// 同一个文件再回退一次才能重新弹出。
    Timer {
        id: noticeTimer
        interval: 3000
        onTriggered: {
            toast.shown = false
            if (toast.backend !== null)
                toast.backend.dismissPlayNotice()
        }
    }

    Component.onCompleted: toast.sync()

    Connections {
        target: toast.backend

        function onChanged() {
            toast.sync()
        }
    }

    onInlineLoadingChanged: toast.sync()

    /// 按后端当前属性决定状态；只在状态真的变了时动定时器 / 显隐。
    function sync() {
        if (toast.backend === null) {
            apply("idle")
            return
        }
        var key = "idle"
        if (toast.failed)
            key = "error:" + toast.backend.playError
        else if (toast.backend.playLoading && !toast.inlineLoading)
            key = "loading:" + toast.backend.playLoadingName
        else if (toast.notice)
            key = "notice:" + toast.backend.playNotice
        apply(key)
    }

    function apply(key) {
        if (key === toast.stateKey)
            return
        toast.stateKey = key
        showTimer.stop()
        noticeTimer.stop()
        if (key.startsWith("error:")) {
            // 错误立即出现，不等 400 ms
            toast.shown = true
        } else if (key.startsWith("loading:")) {
            toast.shown = false
            showTimer.restart()
        } else if (key.startsWith("notice:")) {
            toast.shown = true
            noticeTimer.restart()
        } else {
            toast.shown = false
        }
    }

    /// 提示条上唯一的按钮：加载时是「取消」，错误时是「关闭」。
    function activate() {
        if (toast.backend === null)
            return
        if (toast.loading)
            toast.backend.cancelPlayLoading()
        else if (toast.failed)
            toast.backend.dismissPlayError()
        // invokable 是同步执行的：后端属性已经更新，这里立刻复核一次，
        // 不必等下一个 changed 信号（界面不会多闪一帧）
        sync()
    }

    RowLayout {
        id: layout
        anchors.fill: parent
        anchors.margins: toast.padding
        spacing: 10

        // 转圈：与文件页同一种（BusyIndicator 不会刷 breeze 的 ProgressBar 告警）
        BusyIndicator {
            id: spinner
            visible: toast.loading
            running: visible
            Layout.alignment: Qt.AlignVCenter
            Layout.preferredWidth: 18
            Layout.preferredHeight: 18
        }

        ElidedLabel {
            id: message
            Layout.maximumWidth: toast.maxTextWidth
            Layout.alignment: Qt.AlignVCenter
            elide: Text.ElideRight
            font.pixelSize: theme.fontMeta
            color: toast.failed ? theme.danger : theme.textPrimary
            // 文件名与错误文案都是外部数据：按纯文本渲染，不当富文本
            textFormat: Text.PlainText
            text: toast.failed ? toast.backend.playError
                  : toast.loading ? qsTr("正在加载「%1」…").arg(toast.backend.playLoadingName)
                  : toast.notice ? toast.backend.playNotice
                  : ""
        }

        KButton {
            objectName: "playToastAction"
            Layout.alignment: Qt.AlignVCenter
            visible: toast.loading || toast.failed
            text: toast.loading ? qsTr("取消") : qsTr("关闭")
            variant: "secondary"
            implicitHeight: 28
            onClicked: toast.activate()
        }
    }
}
