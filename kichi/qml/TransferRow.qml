import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 传输任务页的一行（从 TransfersPage.qml 拆出来：那里已过行数软上限）。
//
// 每行固定 `implicitHeight: 60`，第二行是固定 20px 的槽位（下载中放进度条，
// 其余放说明，都没有就留白）。否则「下载中 / 准备中 / 排队 / 完成 / 失败」
// 各自算高，列表参差不齐（docs/pitfalls.md 第 8 条）。
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

    /// 这一行是否刚被定位（约 1.2 秒后由页面清掉）
    property bool flash: false
    /// 实时进度（下载中那一条由 transferLive 单独刷新）
    property var live: null
    /// 页面把 Kichi 后端传进来（本组件不直接引 backend 单例）
    property var backend: null

    readonly property bool notice: row.phase === "notice"
    readonly property bool failed: row.phase === "failed"
    readonly property bool preparing: row.phase === "preparing"
    readonly property bool canReveal: row.phase === "done" && row.path.length > 0

    /// 在文件管理器里选中落盘文件（整段路径与「打开位置」都走这里）
    function revealFile() {
        if (row.canReveal && row.backend !== null)
            row.backend.revealTransfer(row.key)
    }

    /// 重新入队这一条失败记录（记录表由 Rust 侧处理，本组件只发请求）
    function retryDownload() {
        if (row.failed && row.backend !== null)
            row.backend.retryTransfer(row.key)
    }

    // 行高写死（与文件列表同一做法）：交给控件自己算会让各状态高低不一
    implicitHeight: 60
    padding: 8
    leftPadding: 10
    rightPadding: 10

    /// 某一行当前该显示的进度（实时进度优先）
    function liveProgress(fallback) {
        return row.live !== null && row.live.key === row.key ? row.live.progress : fallback
    }

    function liveProgressText(fallback) {
        return row.live !== null && row.live.key === row.key ? row.live.progressText : fallback
    }

    function liveSizeText(fallback) {
        return row.live !== null && row.live.key === row.key ? row.live.sizeText : fallback
    }

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

        RowLayout {
            Layout.fillWidth: true
            Layout.preferredHeight: 20
            spacing: 10

            Label {
                visible: !row.notice
                text: row.iconFor(row.name)
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
                text: row.liveSizeText(row.sizeText)
                font.pixelSize: 12
                verticalAlignment: Text.AlignVCenter
                color: palette.placeholderText
                horizontalAlignment: Text.AlignRight
                Layout.preferredWidth: 150
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

            // 说明行 + 行内动作：整段路径可点（完成）、右端「打开位置」/「重试」。
            // 不用 Button / ToolButton：Breeze 的 padding 会撑破 20px 槽位。
            RowLayout {
                anchors.fill: parent
                spacing: 10
                visible: !row.running

                Label {
                    id: detailLabel
                    objectName: "detailLink"
                    Layout.fillWidth: true
                    // 准备中单独一条说明：读目录期间没有进度也没有落盘路径
                    text: row.preparing ? qsTr("正在读取目录结构…") : row.detail
                    font.pixelSize: 12
                    verticalAlignment: Text.AlignVCenter
                    elide: Text.ElideMiddle
                    color: row.failed ? "#c0392b" : palette.placeholderText
                    font.underline: detailMouse.containsMouse && row.canReveal

                    MouseArea {
                        id: detailMouse
                        anchors.fill: parent
                        enabled: row.canReveal
                        hoverEnabled: true
                        cursorShape: row.canReveal ? Qt.PointingHandCursor : Qt.ArrowCursor
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
                    visible: row.failed
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
