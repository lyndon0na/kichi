import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 传输任务页：本次运行内的下载记录（排队 / 下载中 / 完成 / 失败 / 取消）。
//
// 数据全部来自 backend.transfers（Rust 侧已排好序：进行中在前，已结束新→旧），
// 本页只负责显示与三个操作入口。记录不落盘，退出 kichi 即清空。
Item {
    id: page

    property var backend: null

    /// 记录的结构版本：Rust 只在结构变化时 +1。
    /// 进度节拍只更新 transferLive（见 below），不碰列表。
    property int revision: -1

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

    /// 把 Rust 的快照贴到 ListModel 上（只动差异，不整表重灌）。
    ///
    /// 关键：一次结构变化只做**最少几次** move，不能逐格挪。逐格把每一行
    /// 拽到目标位置时，一次完成会产生上百次 move，ListView 的内部位置会被
    /// 搅乱（`contentY` 与真正渲染的行对不上），随后被强制重排、跳到底部
    /// （2026-10-27 用户实测：滚到中间，每完成一项跳一次）。
    ///
    /// 做法：从头扫一遍，站错位置的那一行直接 move 到它的最终位置；
    /// 不在快照里的删掉；扫完把缺的补到末尾。
    ///
    /// 角色名不能叫 `state`（`Item` 自带同名属性，qmllint 报 property-override，
    /// 运行时也读不到角色值），因此统一映射成 `phase`。
    function syncRows() {
        if (!page.backend)
            return
        var wanted = page.backend.transfers
        var wantedIndex = {}
        for (var i = 0; i < wanted.length; i++)
            wantedIndex[wanted[i].key] = i

        var index = 0
        while (index < wanted.length) {
            if (index < rows.count && rows.get(index).key === wanted[index].key) {
                index++
                continue
            }
            if (index < rows.count) {
                var rowKey = rows.get(index).key
                if (wantedIndex[rowKey] === undefined) {
                    rows.remove(index)
                    continue
                }
                // 这一行属于快照但站错了位置：直接放到它该在的地方
                rows.move(index, wantedIndex[rowKey], 1)
                continue
            }
            rows.insert(index, normalize(wanted[index]))
            index++
        }
        while (rows.count > wanted.length)
            rows.remove(rows.count - 1)

        // 位置对上了，再补字段差异（状态 / 大小 / 说明）
        for (var target = 0; target < wanted.length && target < rows.count; target++) {
            var item = normalize(wanted[target])
            var current = rows.get(target)
            for (var field in item) {
                if (current[field] !== item[field])
                    rows.setProperty(target, field, item[field])
            }
        }
    }

    function normalize(item) {
        return {
            key: item.key,
            name: item.name,
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

    /// 进行中条数（排队 + 下载中）
    readonly property int activeCount: page.backend ? page.backend.downloadActive : 0

    /// 正在下载那一条的实时进度（结构变化时它会并进 transfers 模型）
    readonly property var liveRow: {
        var live = page.backend ? page.backend.transferLive : []
        return live.length > 0 ? live[0] : null
    }

    /// 某一行当前该显示的进度（实时进度优先）
    function liveProgress(key, fallback) {
        return page.liveRow !== null && page.liveRow.key === key
               ? page.liveRow.progress : fallback
    }

    function liveProgressText(key, fallback) {
        return page.liveRow !== null && page.liveRow.key === key
               ? page.liveRow.progressText : fallback
    }

    function liveSizeText(key, fallback) {
        return page.liveRow !== null && page.liveRow.key === key
               ? page.liveRow.sizeText : fallback
    }

    /// 按扩展名给行配图标：传输记录只有路径，没有条目类型
    function iconFor(name) {
        var lower = String(name || "").toLowerCase()
        if (/\.(mp4|mkv|avi|mov|webm|ts|m2ts|flv|wmv|rmvb)$/.test(lower))
            return "🎬"
        if (/\.(mp3|flac|wav|m4a|ogg|opus|aac|ape)$/.test(lower))
            return "🎵"
        if (/\.(jpg|jpeg|png|gif|webp|bmp|heic|avif)$/.test(lower))
            return "🖼️"
        return "📄"
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
                text: page.activeCount > 0 ? qsTr("%1 个进行中").arg(page.activeCount)
                                           : qsTr("没有进行中的传输")
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

            delegate: ItemDelegate {
                id: row
                required property string key
                required property string name
                required property string phase
                required property string stateText
                required property bool running
                required property int progress
                required property string progressText
                required property string sizeText
                required property string detail

                readonly property bool notice: row.phase === "notice"
                readonly property bool failed: row.phase === "failed"

                // 行宽减去滚动条车道：条贴窗口边，文字停在线左边
                width: ListView.view.width - metrics.scrollBarSlot
                padding: 8
                leftPadding: 10
                rightPadding: 10
                // 行高写死（与文件列表同一做法，见 docs/pitfalls.md #8）：
                // 第二行有时是进度条、有时是说明、有时没有内容，交给控件自己算
                // 会让四种状态高低不一。这里固定「第一行 + 第二行」的总高。
                implicitHeight: 60

                background: Rectangle {
                    radius: 4
                    color: row.hovered ? Qt.rgba(palette.windowText.r, palette.windowText.g,
                                                 palette.windowText.b, 0.05) : "transparent"
                }

                contentItem: ColumnLayout {
                    spacing: 4

                    RowLayout {
                        Layout.fillWidth: true
                        Layout.preferredHeight: 20
                        spacing: 10

                        Label {
                            visible: !row.notice
                            text: page.iconFor(row.name)
                            font.pixelSize: 18
                            Layout.preferredWidth: 22
                            horizontalAlignment: Text.AlignHCenter
                            verticalAlignment: Text.AlignVCenter
                        }

                        Label {
                            Layout.fillWidth: true
                            text: row.name
                            elide: Text.ElideMiddle
                            verticalAlignment: Text.AlignVCenter
                            color: row.notice ? palette.placeholderText : palette.text
                        }

                        Label {
                            visible: !row.notice
                            text: row.stateText
                            font.pixelSize: 12
                            verticalAlignment: Text.AlignVCenter
                            color: row.failed ? "#c0392b"
                                 : row.phase === "done" ? "#3c9d5a"
                                 : palette.placeholderText
                        }

                        Label {
                            visible: !row.notice
                            text: page.liveSizeText(row.key, row.sizeText)
                            font.pixelSize: 12
                            verticalAlignment: Text.AlignVCenter
                            color: palette.placeholderText
                            horizontalAlignment: Text.AlignRight
                            Layout.preferredWidth: 150
                        }
                    }

                    // 第二行：固定高度。下载中放进度条，其余放一行说明
                    // （落盘路径 / 失败原因），都没有时留同样高的空白。
                    Item {
                        Layout.fillWidth: true
                        Layout.preferredHeight: 20
                        // 与第一行的名字对齐（图标 22 + 间距 10）
                        Layout.leftMargin: 32

                        RowLayout {
                            anchors.fill: parent
                            spacing: 10
                            visible: row.running

                            ProgressBar {
                                Layout.fillWidth: true
                                from: 0
                                to: 100
                                value: Math.max(page.liveProgress(row.key, row.progress), 0)
                                indeterminate: page.liveProgress(row.key, row.progress) < 0
                            }

                            Label {
                                text: page.liveProgressText(row.key, row.progressText)
                                font.pixelSize: 12
                                verticalAlignment: Text.AlignVCenter
                                color: palette.placeholderText
                            }
                        }

                        Label {
                            anchors.fill: parent
                            visible: !row.running && row.detail.length > 0
                            text: row.detail
                            font.pixelSize: 12
                            verticalAlignment: Text.AlignVCenter
                            elide: Text.ElideMiddle
                            color: row.failed ? "#c0392b" : palette.placeholderText
                        }
                    }
                }
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
