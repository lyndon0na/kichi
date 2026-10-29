import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 传输任务页的一行（从 TransfersPage.qml 拆出来：那里已过行数软上限）。
//
// 每行固定 `implicitHeight: 60`，第二行是固定 20px 的槽位（下载中放进度条，
// 其余放说明，都没有就留白）。否则「下载中 / 准备中 / 排队 / 完成 / 失败」
// 各自算高，列表参差不齐（docs/pitfalls.md 第 8 条）。
//
// 文件夹批次在主列表只占一行：`folder=true`，`childCount / failedCount`
// 由 Rust 聚合；点击第一行进入文件夹详情页。子文件行只出现在详情页，
// 它们在这里就是普通文件行（`folder=false`）。
//
// 角色名不能叫 `state` 或 `highlighted`：`ItemDelegate` 自带同名属性，
// 静态检查报 property-override、运行时也读不到值（同一条第 12 条）。
ItemDelegate {
    id: row

    // —— 数据（来自 Rust 的 transfers 快照，required 才有增量通知）——
    required property string key
    required property string name
    required property string sourceId
    required property string phase
    required property string stateText
    required property bool running
    required property int progress
    required property string progressText
    required property string sizeText
    required property string detail
    /// 完成行的落盘路径（未完成 / 失败为空串）
    required property string path
    /// 是否文件夹批次的总账行
    required property bool folder
    /// 文件夹批次的子文件数（其他行为 0）
    required property int childCount
    /// 文件夹批次失败的子文件数（其他行为 0）
    required property int failedCount

    /// 点文件夹行第一行（或右侧 ›）时发出去，由页面决定进详情
    signal openDetailsRequested(string key)

    /// 这一行是否刚被定位（约 1.2 秒后由页面清掉）
    property bool flash: false
    /// 按 key 的实时进度 map（下载中那一行由 transferLive 单独刷新）
    property var live: null
    /// 页面把 Kichi 后端传进来（本组件不直接引 backend 单例）
    property var backend: null

    KTheme { id: theme }

    readonly property bool notice: row.phase === "notice"
    readonly property bool failed: row.phase === "failed"
    /// 上次退出时没跑完（读回的历史行）：给「重试」，不给「打开位置」
    readonly property bool interrupted: row.phase === "interrupted"
    /// 空文件夹 Done 行没有子行但有落盘路径（path 非空），所以下面两条要分开看：
    /// 没有子行 → 不显示下钻箭头；有路径 → 仍可「打开位置」。
    readonly property bool canOpenDetails: row.folder && row.childCount > 0
    readonly property bool hasFailedChildren: row.folder && row.failedCount > 0
    /// 只有真的落过盘才有位置可打开（已中断 / 失败 / 取消都没有）
    readonly property bool canReveal: row.path.length > 0
    /// 子文件失败 / 单文件失败可重试；文件夹父行只有「读目录前整体失败或整个中断」
    /// （没有任何子行）才能整批重试，部分失败去详情页逐个重试。
    /// 已中断的行同样给「重试」：断点还在就续传，改过下载目录就从头下。
    readonly property bool canRetry: (row.failed || row.interrupted)
                                     && (!row.folder || row.childCount === 0)

    /// 在文件管理器里选中落盘文件 / 打开文件夹（整段路径与「打开位置」都走这里）
    function revealFile() {
        if (row.canReveal && row.backend !== null)
            row.backend.revealTransfer(row.key)
    }

    /// 重新入队这一条失败记录（记录表由 Rust 侧处理，本组件只发请求）
    function retryDownload() {
        if (row.canRetry && row.backend !== null)
            row.backend.retryTransfer(row.key)
    }

    function openDetails() {
        if (row.canOpenDetails)
            row.openDetailsRequested(row.key)
    }

    /// 某一行当前该显示的进度（按 key 查实时进度，没有就用快照里的值）
    function liveField(name, fallback) {
        if (row.live === null || row.live === undefined)
            return fallback
        var entry = row.live[row.key]
        if (entry === undefined || entry === null)
            return fallback
        var value = entry[name]
        return value === undefined ? fallback : value
    }

    function liveProgress(fallback) {
        return row.liveField("progress", fallback)
    }

    function liveProgressText(fallback) {
        return row.liveField("progressText", fallback)
    }

    function liveSizeText(fallback) {
        return row.liveField("sizeText", fallback)
    }

    /// 速度 + 剩余时间，或「已卡住 N 秒」（只有 transferLive 带这个键）
    function liveRateText() {
        return row.liveField("rateText", "")
    }

    /// 下载中的重试 / 续传说明（恢复后 5 秒由 Rust 清空）
    function liveNoteText() {
        return row.liveField("noteText", "")
    }

    // 行高写死（与文件列表同一做法）：交给控件自己算会让各状态高低不一。
    // 68 = 上下 12 + 第一行 22 + 间距 2 + 第二行 20（12 的上下留白正好是
    // 「行内上下 12px」的规范）。
    implicitHeight: 72
    padding: 12
    leftPadding: 14
    rightPadding: 14

    background: Rectangle {
        radius: theme.radiusCard
        // 每行是一张浅色卡片：轻微表面差 + 极浅描边，取代原来的硬边框。
        // 悬停即时生效；「定位到这一条」的高亮单独一层淡入淡出，
        // 免得两种状态混在一个动画里互相拖慢。
        color: row.hovered ? theme.surfaceHover : theme.surfaceRaised
        border.width: 1
        border.color: theme.hairline

        Rectangle {
            anchors.fill: parent
            radius: parent.radius
            color: theme.accentSoft
            border.width: 1
            border.color: theme.softTint(theme.accent, 0.45)
            opacity: row.flash ? 1 : 0

            Behavior on opacity {
                NumberAnimation { duration: 180 }
            }
        }
    }

    contentItem: ColumnLayout {
        spacing: 4

        // 第一行整行可点：文件夹且有子文件时进详情页
        Item {
            Layout.fillWidth: true
            Layout.preferredHeight: 22

            RowLayout {
                anchors.fill: parent
                spacing: 10

                // 图标列固定 22：与文件列表用的是同一个 FileIcon，
                // 传输记录只有路径、没有条目类型，所以只按扩展名判断
                Item {
                    visible: !row.notice
                    Layout.preferredWidth: 22
                    Layout.preferredHeight: 20

                    FileIcon {
                        width: 18
                        height: 18
                        anchors.centerIn: parent
                        kind: row.folder ? "folder" : ""
                        fileName: row.name
                        color: palette.text
                        opacity: row.folder ? 1.0 : 0.6
                    }
                }

                Label {
                    Layout.fillWidth: true
                    text: row.name
                    elide: Text.ElideMiddle
                    verticalAlignment: Text.AlignVCenter
                    font.weight: row.folder ? Font.DemiBold : Font.Normal
                    color: row.notice ? theme.textMuted : theme.textPrimary
                }

                // 速度 / 剩余时间 / 卡住：在名称与状态之间固定占位。
                // 放在大小列右侧的话，下载结束速度变空时状态 / 大小 / › 会横向跳动；
                // 这里只让名称让出 190px（只在下载中显示，其余状态不占位）。
                Label {
                    objectName: "rateText"
                    visible: row.running
                    text: row.liveRateText()
                    font.pixelSize: theme.fontMeta
                    verticalAlignment: Text.AlignVCenter
                    color: theme.textMuted
                    horizontalAlignment: Text.AlignRight
                    elide: Text.ElideRight
                    Layout.preferredWidth: 190
                    Layout.minimumWidth: 110
                    Layout.maximumWidth: 190
                }

                KStatusBadge {
                    objectName: "stateText"
                    visible: !row.notice
                    text: row.stateText
                    tone: row.failed ? "danger"
                        : row.interrupted ? "warning"
                        : row.phase === "done" ? "success"
                        : row.running ? "accent"
                        : "neutral"
                }

                Label {
                    visible: !row.notice
                    objectName: "sizeText"
                    text: row.liveSizeText(row.sizeText)
                    font.pixelSize: theme.fontMeta
                    verticalAlignment: Text.AlignVCenter
                    color: theme.textMuted
                    horizontalAlignment: Text.AlignRight
                    Layout.preferredWidth: 150
                }

                // 文件夹行的下钻提示；没有子文件（排队 / 准备中 / 空文件夹）不显示
                Label {
                    objectName: "openDetailsHint"
                    visible: row.canOpenDetails
                    text: "›"
                    font.pixelSize: 14
                    Layout.preferredWidth: 12
                    horizontalAlignment: Text.AlignHCenter
                    verticalAlignment: Text.AlignVCenter
                    color: theme.textMuted
                }
            }

            MouseArea {
                objectName: "rowHeader"
                anchors.fill: parent
                enabled: row.canOpenDetails
                hoverEnabled: enabled
                acceptedButtons: Qt.LeftButton
                cursorShape: enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
                onClicked: row.openDetails()
            }
        }

        // 第二行：固定高度。下载中放进度条，其余放一行说明
        // （准备中提示 / 落盘路径 / 失败原因），都没有时留同样高的空白。
        Item {
            Layout.fillWidth: true
            Layout.preferredHeight: 20
            // 与第一行的名字对齐（图标 22 + 间距 10）
            Layout.leftMargin: 32

            RowLayout {
                anchors.fill: parent
                spacing: 10
                visible: row.running

                KProgressBar {
                    objectName: "rowProgress"
                    Layout.fillWidth: true
                    // 窄窗口下别被右侧文案挤成 0
                    Layout.minimumWidth: 40
                    from: 0
                    to: 100
                    value: Math.max(row.liveProgress(row.progress), 0)
                    indeterminate: row.liveProgress(row.progress) < 0
                }

                Label {
                    text: row.liveProgressText(row.progressText)
                    font.pixelSize: theme.fontMeta
                    verticalAlignment: Text.AlignVCenter
                    color: theme.textMuted
                }

                // 重试 / 续传说明：以前只在快照 detail 里、被下载中的第二行隐藏，
                // 实际看不见；现在跟着 transferLive 一起贴到这一行。
                Label {
                    objectName: "noteText"
                    visible: text.length > 0
                    text: row.liveNoteText()
                    font.pixelSize: theme.fontMeta
                    verticalAlignment: Text.AlignVCenter
                    color: theme.textMuted
                    elide: Text.ElideMiddle
                    Layout.maximumWidth: 320
                }
            }

            // 说明行 + 行内动作：整段路径可点（仅单文件 / 子文件），
            // 右端「打开位置」/「重试」。不用 Button / ToolButton：
            // Breeze 的 padding 会撑破 20px 槽位。
            RowLayout {
                anchors.fill: parent
                spacing: 10
                visible: !row.running

                Label {
                    id: detailLabel
                    objectName: "detailLink"
                    Layout.fillWidth: true
                    // 准备中 / 聚合说明都由 Rust 给；文件夹部分失败补一句可点提示
                    text: row.detail + (row.hasFailedChildren ? qsTr("（点开可重试）") : "")
                    font.pixelSize: theme.fontMeta
                    verticalAlignment: Text.AlignVCenter
                    elide: Text.ElideMiddle
                    color: row.failed || row.hasFailedChildren ? theme.danger
                         : row.interrupted ? theme.warning
                         : theme.textMuted
                    font.underline: detailMouse.containsMouse
                                    && row.canReveal && !row.folder

                    MouseArea {
                        id: detailMouse
                        anchors.fill: parent
                        enabled: row.canReveal && !row.folder
                        hoverEnabled: true
                        cursorShape: enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
                        onClicked: row.revealFile()
                    }
                }

                Label {
                    objectName: "revealLink"
                    visible: row.canReveal
                    text: qsTr("打开位置")
                    font.pixelSize: theme.fontMeta
                    verticalAlignment: Text.AlignVCenter
                    color: theme.accent
                    font.underline: revealMouse.containsMouse

                    MouseArea {
                        id: revealMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: row.revealFile()
                    }
                }

                Label {
                    objectName: "retryLink"
                    visible: row.canRetry
                    text: qsTr("重试")
                    font.pixelSize: theme.fontMeta
                    verticalAlignment: Text.AlignVCenter
                    color: theme.accent
                    font.underline: retryMouse.containsMouse

                    // 已中断行：说清断点前提（改过下载目录 = 从头下，ADR 0011 / 0014）
                    ToolTip.visible: row.interrupted && retryMouse.containsMouse
                    ToolTip.delay: 400
                    ToolTip.text: qsTr("上次未完成，若下载目录未变会从断点续传")

                    MouseArea {
                        id: retryMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: row.retryDownload()
                    }
                }
            }
        }
    }
}
