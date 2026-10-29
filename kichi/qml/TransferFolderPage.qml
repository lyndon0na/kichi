import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 文件夹下载的独立详情页（从传输任务主列表下钻）。
//
// 主列表只显示文件夹批次的总账行；本页显示该批次的子文件。
// 文件顺序固定为下载顺序（index 升序），完成一个文件只改这一行，不移动行。
// 本页与主列表都常驻在 TransfersPage 的 StackLayout 里：不用 Loader，
// 返回时主列表的 contentY 不会被重建（见 docs/pitfalls.md）。
Item {
    id: page
    objectName: "transferFolderPage"

    property var backend: null
    /// 当前文件夹行的 key（batch:folder）
    property string folderKey: ""
    /// 主列表里复制过来的文件夹行对象（父页面按 key 找到）
    property var folder: null
    /// 按 key 的实时进度 map（主列表与详情页共用）
    property var live: null
    /// 是否当前显示
    property bool active: false
    /// 父页面的 normalize 函数（主列表与详情页共用字段映射）
    property var normalizeRow: null

    signal backRequested()

    /// 记录的结构版本：只在本页 active 时跟踪（进度节拍不重建列表）
    property int revision: -1

    UiMetrics {
        id: metrics
    }

    KTheme { id: theme }

    ListModel {
        id: detailRows
    }

    Connections {
        target: page.backend

        function onChanged() {
            if (!page.backend || !page.active)
                return
            if (page.backend.transferRevision !== page.revision)
                page.syncDetailRows()
        }
    }

    onActiveChanged: {
        if (page.active) {
            page.revision = -1
            page.syncDetailRows()
            Qt.callLater(page.scrollToTop)
        }
    }

    onFolderKeyChanged: {
        if (page.active) {
            page.revision = -1
            page.syncDetailRows()
            Qt.callLater(page.scrollToTop)
        }
    }

    function scrollToTop() {
        list.positionViewAtBeginning()
    }

    /// 从字段里取头部数据（folder 为 null 时给安全的默认值）
    function folderValue(name, fallback) {
        if (page.folder === null || page.folder === undefined)
            return fallback
        var value = page.folder[name]
        return value === undefined ? fallback : value
    }

    function liveField(name, fallback) {
        if (page.live === null || page.live === undefined)
            return fallback
        var entry = page.live[page.folderKey]
        if (entry === undefined || entry === null)
            return fallback
        var value = entry[name]
        return value === undefined ? fallback : value
    }

    readonly property var liveProgress: page.liveField("progress", page.folderValue("progress", 0))
    readonly property string liveProgressText: page.liveField("progressText", page.folderValue("progressText", ""))
    readonly property string liveSizeText: page.liveField("sizeText", page.folderValue("sizeText", ""))
    /// 整批速度 + 剩余时间 / 卡住（只有 transferLive 带这个键）
    readonly property string folderRateText: page.liveField("rateText", "")

    /// 把 Rust 的详情快照贴到 ListModel 上：只做增删改，不移动、不重排。
    ///
    /// 详情顺序固定为下载顺序；一条子行完成时只 setProperty，滚动位置不动。
    function syncDetailRows() {
        if (!page.backend)
            return
        var wanted = page.backend.transferDetail || []
        var wantedKeys = {}
        for (var i = 0; i < wanted.length; i++)
            wantedKeys[wanted[i].key] = true

        for (var stale = detailRows.count - 1; stale >= 0; stale--) {
            if (wantedKeys[detailRows.get(stale).key] === undefined)
                detailRows.remove(stale)
        }

        var present = {}
        for (var p = 0; p < detailRows.count; p++)
            present[detailRows.get(p).key] = true
        for (var add = 0; add < wanted.length; add++) {
            if (present[wanted[add].key] === undefined) {
                detailRows.insert(Math.min(add, detailRows.count),
                                  page.normalize(wanted[add]))
                present[wanted[add].key] = true
            }
        }

        for (var at = 0; at < wanted.length && at < detailRows.count; at++) {
            var item = page.normalize(wanted[at])
            var current = detailRows.get(at)
            for (var field in item) {
                if (current[field] !== item[field])
                    detailRows.setProperty(at, field, item[field])
            }
        }

        while (detailRows.count > wanted.length)
            detailRows.remove(detailRows.count - 1)

        page.revision = page.backend.transferRevision
    }

    function normalize(item) {
        return page.normalizeRow ? page.normalizeRow(item) : item
    }

    ColumnLayout {
        anchors.fill: parent
        // 左边 / 下边由 TransfersPage 的外层边距负责（本页在 StackLayout 单元格里），
        // 再补一次会与主列表错开；顶部留一点呼吸空间给头部。
        anchors.leftMargin: 0
        anchors.topMargin: metrics.pageMargin
        anchors.bottomMargin: 0
        spacing: theme.gapSm

        // —— 头部第一行：返回 / 文件夹名 / 文件数 / 聚合状态 ——
        RowLayout {
            Layout.fillWidth: true
            Layout.rightMargin: metrics.pageMargin
            spacing: 10

            Label {
                objectName: "backLink"
                text: qsTr("‹ 返回传输任务")
                color: theme.accent
                font.pixelSize: theme.fontMeta
                verticalAlignment: Text.AlignVCenter
                font.underline: backMouse.containsMouse

                MouseArea {
                    id: backMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: page.backRequested()
                }
            }

            Label {
                Layout.fillWidth: true
                text: page.folderValue("name", "")
                font.pixelSize: theme.fontSection
                font.weight: Font.DemiBold
                color: theme.textPrimary
                elide: Text.ElideMiddle
                verticalAlignment: Text.AlignVCenter
            }

            Label {
                text: qsTr("%1 个文件").arg(page.folderValue("childCount", 0))
                font.pixelSize: theme.fontMeta
                verticalAlignment: Text.AlignVCenter
                color: theme.textMuted
            }

            KStatusBadge {
                objectName: "detailStateText"
                text: page.folderValue("stateText", "")
                tone: page.folderValue("phase", "") === "failed" ? "danger"
                    : page.folderValue("phase", "") === "interrupted" ? "warning"
                    : page.folderValue("phase", "") === "done" ? "success"
                    : "neutral"
            }
        }

        // —— 头部第二行：聚合进度 / 聚合说明 + 大小 + 打开位置 ——
        RowLayout {
            Layout.fillWidth: true
            Layout.rightMargin: metrics.pageMargin
            spacing: 10
            visible: page.folder !== null

            KProgressBar {
                visible: page.folderValue("running", false)
                Layout.fillWidth: true
                from: 0
                to: 100
                value: Math.max(page.liveProgress, 0)
                indeterminate: page.liveProgress < 0
            }

            Label {
                visible: page.folderValue("running", false)
                text: page.liveProgressText
                font.pixelSize: theme.fontMeta
                verticalAlignment: Text.AlignVCenter
                color: theme.textMuted
            }

            // 整批速度 + ETA / 卡住：紧跟百分比，窄窗口省略
            Label {
                objectName: "folderRateText"
                visible: page.folderValue("running", false) && text.length > 0
                text: page.folderRateText
                font.pixelSize: theme.fontMeta
                verticalAlignment: Text.AlignVCenter
                color: theme.textMuted
                elide: Text.ElideRight
                Layout.maximumWidth: 260
            }

            Label {
                visible: !page.folderValue("running", false)
                Layout.fillWidth: true
                text: page.folderValue("detail", "")
                font.pixelSize: theme.fontMeta
                elide: Text.ElideMiddle
                verticalAlignment: Text.AlignVCenter
                color: page.folderValue("phase", "") === "failed" ? theme.danger
                     : theme.textMuted
            }

            Label {
                objectName: "detailSizeText"
                text: page.liveSizeText
                font.pixelSize: theme.fontMeta
                verticalAlignment: Text.AlignVCenter
                color: theme.textMuted
            }

            Label {
                objectName: "revealFolderLink"
                visible: page.folderValue("path", "").length > 0
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
                    onClicked: {
                        if (page.folder !== null && page.backend !== null)
                            page.backend.revealTransfer(page.folderKey)
                    }
                }
            }
        }

        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: 1
            Layout.rightMargin: metrics.pageMargin
            color: theme.divider
        }

        ListView {
            id: list
            objectName: "transferDetailList"
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            spacing: 10
            model: detailRows
            boundsBehavior: Flickable.StopAtBounds
            cacheBuffer: 400

            ScrollBar.vertical: KScrollBar {}

            delegate: TransferRow {
                width: list.width - metrics.scrollBarSlot
                live: page.live
                backend: page.backend
                folder: false
                childCount: 0
                failedCount: 0
            }

            Label {
                anchors.centerIn: parent
                visible: list.count === 0
                text: qsTr("没有可显示的文件")
                color: theme.textMuted
                font.pixelSize: theme.fontBody
            }
        }
    }
}
