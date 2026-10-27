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

    readonly property bool notice: row.phase === "notice"
    readonly property bool failed: row.phase === "failed"
    readonly property bool canOpenDetails: row.folder && row.childCount > 0
    readonly property bool hasFailedChildren: row.folder && row.failedCount > 0
    readonly property bool canReveal: row.path.length > 0
    /// 子文件失败 / 单文件失败可重试；文件夹父行只有「读目录前整体失败」
    /// （没有任何子行）才能整批重试，部分失败去详情页逐个重试。
    readonly property bool canRetry: row.failed && (!row.folder || row.childCount === 0)

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

    // 行高写死（与文件列表同一做法）：交给控件自己算会让各状态高低不一
    implicitHeight: 60
    padding: 8
    leftPadding: 10
    rightPadding: 10

    background: Rectangle {
        radius: 4
        // 定位高亮：约 1.2 秒后淡出（页面的 Timer 清掉 flash）
        color: row.flash
               ? Qt.rgba(palette.highlight.r, palette.highlight.g,
                         palette.highlight.b, 0.30)
               : row.hovered ? Qt.rgba(palette.windowText.r, palette.windowText.g,
                                       palette.windowText.b, 0.05)
               : "transparent"

        Behavior on color {
            ColorAnimation { duration: 260 }
        }
    }

    contentItem: ColumnLayout {
        spacing: 4

        // 第一行整行可点：文件夹且有子文件时进详情页
        Item {
            Layout.fillWidth: true
            Layout.preferredHeight: 20

            RowLayout {
                anchors.fill: parent
                spacing: 10

                Label {
                    visible: !row.notice
                    text: row.folder ? "📁" : row.iconFor(row.name)
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
                    font.bold: row.folder
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
                    objectName: "sizeText"
                    text: row.liveSizeText(row.sizeText)
                    font.pixelSize: 12
                    verticalAlignment: Text.AlignVCenter
                    color: palette.placeholderText
                    horizontalAlignment: Text.AlignRight
                    Layout.preferredWidth: 150
                }

                // 文件夹行的下钻提示；没有子文件（排队 / 准备中 / 空目录失败）不显示
                Label {
                    objectName: "openDetailsHint"
                    visible: row.canOpenDetails
                    text: "›"
                    font.pixelSize: 16
                    Layout.preferredWidth: 12
                    horizontalAlignment: Text.AlignHCenter
                    verticalAlignment: Text.AlignVCenter
                    color: palette.placeholderText
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
                    from: 0
                    to: 100
                    value: Math.max(row.liveProgress(row.progress), 0)
                    indeterminate: row.liveProgress(row.progress) < 0
                }

                Label {
                    text: row.liveProgressText(row.progressText)
                    font.pixelSize: 12
                    verticalAlignment: Text.AlignVCenter
                    color: palette.placeholderText
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
                    font.pixelSize: 12
                    verticalAlignment: Text.AlignVCenter
                    elide: Text.ElideMiddle
                    color: row.failed || row.hasFailedChildren
                         ? "#c0392b" : palette.placeholderText
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
                    font.pixelSize: 12
                    verticalAlignment: Text.AlignVCenter
                    color: palette.link
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
                    font.pixelSize: 12
                    verticalAlignment: Text.AlignVCenter
                    color: palette.link
                    font.underline: retryMouse.containsMouse

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
}
