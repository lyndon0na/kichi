import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 传输任务页：本次运行内的下载记录（排队 / 准备中 / 下载中 / 完成 / 失败 / 取消）。
//
// 数据全部来自 backend.transfers（Rust 侧已排好序：进行中在前，已结束新→旧），
// 本页只负责显示与三个操作入口。记录不落盘，退出 kichi 即清空。
Item {
    id: page

    property var backend: null

    /// 记录的结构版本：Rust 只在结构变化时 +1。
    /// 进度节拍只更新 transferLive（见 below），不碰列表。
    property int revision: -1

    /// 要定位到的来源条目 id（由 main.qml 在下载时写入）。
    property string focusSourceId: ""

    /// 当前高亮的行 key（约 1.2 秒后淡出）
    property string highlightedKey: ""

    // 页面尺寸常量：滚动条车道（见 KScrollBar / docs/pitfalls.md 第 13 条）
    UiMetrics {
        id: metrics
    }

    // —— 列表模型 ——
    // ⚠ 不能用「每次拿一个新数组赋给 ListView.model」：那会让 ListView 整表
    // 重灌，滚动位置被冲回顶部；即便事后把位置放回去，中间也会闪一帧列表顶部
    // （那里正是「正在下载」那条的进度条，2026-10-27 用户实测看到闪框）。
    // 这里用 ListModel 就地做差异：移动 / 改字段 / 增删，列表不重建。
    ListModel {
        id: rows
    }

    Connections {
        target: page.backend

        function onChanged() {
            if (!page.backend)
                return
            if (page.backend.transferRevision !== page.revision) {
                page.revision = page.backend.transferRevision
                page.syncRows()
            }
        }
    }

    Component.onCompleted: syncRows()

    // —— 定位（点已占用的条目时跳过来看）——
    /// 请求定位到某个来源条目（main.qml 收到 locateTransfer 后调用）。
    ///
    /// 用**直接调用**而不是监听属性变化：同一个条目连点两次时属性值不变，
    /// changed 信号不会再来一次，但用户仍然期望跳过去再看一眼。
    /// 记录可能刚变（这一下才读到新批次），所以等一帧：
    /// `Qt.callLater` 在 `syncRows` 之后跑。
    function focusSource(sourceId) {
        if (sourceId.length === 0)
            return
        page.focusSourceId = sourceId
        Qt.callLater(page.locate)
    }

    Timer {
        id: highlightTimer
        interval: 1200
        repeat: false
        onTriggered: page.highlightedKey = ""
    }

    /// 滚到 focusSourceId 对应的第一条进行中行并短暂高亮。
    function locate() {
        if (page.focusSourceId.length === 0)
            return
        page.syncRows()
        for (var i = 0; i < rows.count; i++) {
            var row = rows.get(i)
            if (row.sourceId !== page.focusSourceId)
                continue
            if (!page.isActivePhase(row.phase))
                continue
            list.positionViewAtIndex(i, ListView.Contain)
            page.highlightedKey = row.key
            highlightTimer.restart()
            return
        }
        // 找不到 active 行时退而求其次：已经结束的那条也给用户看一眼
        for (var done = 0; done < rows.count; done++) {
            if (rows.get(done).sourceId === page.focusSourceId) {
                list.positionViewAtIndex(done, ListView.Contain)
                page.highlightedKey = rows.get(done).key
                highlightTimer.restart()
                return
            }
        }
    }

    function isActivePhase(phase) {
        return phase === "queued" || phase === "preparing" || phase === "running"
    }

    /// 把 Rust 的快照贴到 ListModel 上（只动差异，不整表重灌）。
    ///
    /// 关键一：一次结构变化只做**最少几次** move。逐格挪会把 `ListView` 的内部
    /// 行位置搅乱，随后被强制重排、跳到别处（2026-10-27 用户实测：滚到中间，
    /// 每完成一项就往下跳一次）。一条记录完成时，它要从「进行中」段挪到
    /// 「已完成」段——正向扫描会把它一格一格往后推（一次完成 = 几十次 move），
    /// 所以这里**从尾往前**对齐：位置正确的行一次都不动，一条完成最多一次 move。
    ///
    /// 关键二：**move 的目标位置必须小于当前 count**，等于 count 会被 Qt 判
    /// `move: out of range` 并直接丢弃（不是 clamp）——游标不前进就是死循环。
    /// 从尾往前时 `target` 只会小于 count，天然不会踩到。
    ///
    /// 做法（每一步都不会越界）：
    /// 1. 倒序删掉不在快照里的行；
    /// 2. 快照里有、模型里没有的行先追加到末尾（位置交给第 3 步修正）；
    /// 3. 从尾往前逐位对：对不上就把这一行从**前面**移过来（后面的位置都已固定，
    ///    所以它必然在前面）；
    /// 4. 多出来的行删掉。
    ///
    /// 角色名不能叫 `state`（`Item` 自带同名属性，qmllint 报 property-override，
    /// 运行时也读不到角色值），因此统一映射成 `phase`。
    function syncRows() {
        if (!page.backend)
            return
        var wanted = page.backend.transfers
        var wantedKeys = {}
        for (var i = 0; i < wanted.length; i++)
            wantedKeys[wanted[i].key] = true

        // 1. 倒序删：边删边倒着走，下标才不会错位
        for (var stale = rows.count - 1; stale >= 0; stale--) {
            if (wantedKeys[rows.get(stale).key] === undefined)
                rows.remove(stale)
        }

        // 2. 缺的行先追加到末尾，保证下一步 count 与快照一致
        var present = {}
        for (var p = 0; p < rows.count; p++)
            present[rows.get(p).key] = true
        for (var add = 0; add < wanted.length; add++) {
            if (present[wanted[add].key] === undefined) {
                rows.append(normalize(wanted[add]))
                present[wanted[add].key] = true
            }
        }

        // 3. 从尾往前对齐：后面的位置已固定，故这一步要的行一定在前面
        for (var target = wanted.length - 1; target >= 0 && target < rows.count; target--) {
            var key = wanted[target].key
            if (rows.get(target).key === key)
                continue
            var from = -1
            for (var scan = target - 1; scan >= 0; scan--) {
                if (rows.get(scan).key === key) {
                    from = scan
                    break
                }
            }
            if (from >= 0)
                rows.move(from, target, 1)
        }

        // 4. 理论上不会有多出来的行；防御性删掉，避免残留
        while (rows.count > wanted.length)
            rows.remove(rows.count - 1)

        // 位置对上了，再补字段差异（状态 / 大小 / 说明）
        for (var at = 0; at < wanted.length && at < rows.count; at++) {
            var item = normalize(wanted[at])
            var current = rows.get(at)
            for (var field in item) {
                if (current[field] !== item[field])
                    rows.setProperty(at, field, item[field])
            }
        }
    }

    function normalize(item) {
        return {
            key: item.key,
            name: item.name,
            sourceId: item.sourceId || "",
            phase: item.state,
            stateText: item.stateText,
            running: item.running,
            progress: item.progress,
            progressText: item.progressText,
            sizeText: item.sizeText,
            detail: item.detail
        }
    }

    /// 有没有可清的已结束记录（按钮据此置灰）
    readonly property bool hasFinished: {
        for (var i = 0; i < rows.count; i++) {
            var phase = rows.get(i).phase
            if (phase === "done" || phase === "failed" || phase === "cancelled")
                return true
        }
        return false
    }

    /// 进行中条数（排队 + 准备中 + 下载中）
    readonly property int activeCount: page.backend ? page.backend.downloadActive : 0

    /// 正在下载那一条的实时进度（结构变化时它会并进 transfers 模型）
    readonly property var liveRow: {
        var live = page.backend ? page.backend.transferLive : []
        return live.length > 0 ? live[0] : null
    }

    ColumnLayout {
        anchors.fill: parent
        // 右边不留：列表要铺到窗口右缘，让滚动条贴边；
        // 工具条 / 分隔线自己补右边距，列表内容宽度减去车道
        anchors.leftMargin: metrics.pageMargin
        anchors.topMargin: metrics.pageMargin
        anchors.bottomMargin: metrics.pageMargin
        spacing: 8

        // —— 工具条：进行中条数 + 三个操作 ——
        RowLayout {
            Layout.fillWidth: true
            // 工具条不吃滚动条车道，右缘留普通页边距（视觉位置与以前一致）
            Layout.rightMargin: metrics.pageMargin
            spacing: 8

            Label {
                Layout.fillWidth: true
                text: {
                    if (page.activeCount === 0)
                        return qsTr("没有进行中的传输")
                    var queued = page.backend ? page.backend.downloadQueued : 0
                    return queued > 0
                        ? qsTr("%1 个进行中 · %2 批排队").arg(page.activeCount).arg(queued)
                        : qsTr("%1 个进行中").arg(page.activeCount)
                }
                color: page.activeCount > 0 ? palette.text : palette.placeholderText
                font.pixelSize: 12
            }

            Button {
                text: qsTr("打开下载目录")
                onClicked: if (page.backend) page.backend.openDownloadDir()
            }

            Button {
                text: qsTr("取消下载")
                enabled: page.activeCount > 0
                onClicked: if (page.backend) page.backend.cancelDownloads()
            }

            Button {
                text: qsTr("清空已完成记录")
                enabled: page.backend !== null && page.hasFinished
                onClicked: if (page.backend) page.backend.clearFinishedTransfers()
            }
        }

        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: 1
            Layout.rightMargin: metrics.pageMargin
            color: Qt.rgba(palette.windowText.r, palette.windowText.g, palette.windowText.b, 0.10)
        }

        ListView {
            id: list
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            spacing: 2
            model: rows
            boundsBehavior: Flickable.StopAtBounds
            cacheBuffer: 400

            // 列表铺到窗口右缘，条画在右侧 10px 车道里
            ScrollBar.vertical: KScrollBar {}

            delegate: TransferRow {
                id: row
                width: list.width - metrics.scrollBarSlot
                flash: row.key === page.highlightedKey
                live: page.liveRow
            }

            Label {
                anchors.centerIn: parent
                visible: list.count === 0
                text: qsTr("暂无传输任务\n在文件页右键选择下载即可开始")
                horizontalAlignment: Text.AlignHCenter
                color: palette.placeholderText
            }
        }
    }
}
