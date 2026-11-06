import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 添加离线任务：每行一个磁力 / ed2k / HTTP 链接。
KDialog {
    id: dialog

    property var backend: null
    /// null 表示官方 My Pack 下载目录；空串表示根目录，其它值表示指定目录 id。
    property var targetFolderId: null
    property string targetFolderPath: ""
    /// 打开选择器时的起始完整路径（根 → 当前）。
    property var pickerStartPath: []
    readonly property bool saveToFolder: targetFolderId !== null
    property string torrentNotice: ""
    property bool torrentNoticeError: false
    /// 第一行无法识别的输入（行号从 1 起）；0 表示没有或输入为空。
    readonly property int invalidLine: dialog.firstInvalidLine(input.text)
    readonly property int linkCount: dialog.countLinks(input.text)
    readonly property bool canSubmit: dialog.backend !== null
                                        && dialog.linkCount > 0
                                        && dialog.invalidLine === 0
    /// 所有非空行都是磁力链接（或裸 infohash）时，添加前先批量解析文件清单。
    readonly property var magnetLinks: dialog.allMagnetLinks(input.text)
    /// 单条磁链正在后台解析；期间锁定添加 / 取消，避免结果落到已关闭的弹窗。
    property bool parsingMagnet: false

    objectName: "newTaskDialog"
    title: qsTr("新任务")
    modal: true
    parent: Overlay.overlay
    anchors.centerIn: Overlay.overlay
    width: 520 + leftPadding + rightPadding
    spacing: theme.gapMd
    closePolicy: dialog.parsingMagnet ? Popup.NoAutoClose : Popup.CloseOnEscape

    onOpened: dialog.resetForOpen()

    function resetForOpen() {
        input.text = ""
        dialog.parsingMagnet = false
        dialog.torrentNotice = ""
        dialog.torrentNoticeError = false
        input.forceActiveFocus()
    }

    footer: DialogFooter {
        outerMargin: dialog.padding
        statusText: dialog.invalidLine > 0
                    ? qsTr("第 %1 行不是有效的磁力 / ed2k / HTTP 链接").arg(dialog.invalidLine)
                    : dialog.torrentNotice.length > 0
                      ? dialog.torrentNotice
                      : qsTr("支持磁力 / infohash / ed2k / HTTP；可拖入或选择 .torrent 文件。")
        statusColor: dialog.invalidLine > 0 || dialog.torrentNoticeError
                     ? theme.danger
                     : theme.textMuted
        okText: dialog.parsingMagnet ? qsTr("解析中…") : qsTr("添加")
        okObjectName: "newTaskDialogOk"
        cancelObjectName: "newTaskDialogCancel"
        okEnabled: dialog.canSubmit && !dialog.parsingMagnet
        cancelEnabled: !dialog.parsingMagnet
        onOkClicked: dialog.submit()
        onCancelClicked: dialog.close()
    }

    ColumnLayout {
        width: 520
        spacing: theme.gapMd

        RowLayout {
            objectName: "newTaskDestinationRow"
            Layout.fillWidth: true
            spacing: theme.gapSm

            NavIcon {
                implicitWidth: 16
                implicitHeight: 16
                Layout.alignment: Qt.AlignVCenter
                name: "folder"
                color: theme.textSecondary
            }

            ElidedLabel {
                objectName: "newTaskDestinationPath"
                Layout.fillWidth: true
                text: dialog.saveToFolder
                      ? qsTr("保存到：%1").arg(
                            dialog.targetFolderPath.length > 0
                            ? dialog.targetFolderPath
                            : qsTr("当前文件夹"))
                      : qsTr("保存到：My Pack")
                color: theme.textSecondary
                font.pixelSize: theme.fontMeta
                elide: Text.ElideMiddle
                verticalAlignment: Text.AlignVCenter
            }

            KButton {
                objectName: "newTaskChangeDestination"
                text: qsTr("更改位置…")
                variant: "ghost"
                implicitHeight: 28
                showTextToolTip: false
                onClicked: folderPicker.openPicker(dialog.pickerStartPath)
            }
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: theme.gapSm

            Label {
                Layout.fillWidth: true
                text: qsTr("每行粘贴一个链接")
                color: theme.textPrimary
                font.pixelSize: theme.fontBody
                font.weight: Font.DemiBold
            }

            KButton {
                objectName: "newTaskChooseTorrent"
                text: qsTr("选择 .torrent 文件…")
                iconName: "torrent"
                iconSize: 14
                variant: "ghost"
                implicitHeight: 28
                leftPadding: 8
                rightPadding: 8
                enabled: dialog.backend !== null
                         && typeof dialog.backend.chooseTorrentFiles === "function"
                onClicked: dialog.chooseTorrentFiles()
            }
        }

        Item {
            Layout.fillWidth: true
            Layout.preferredHeight: 180

            Rectangle {
                anchors.fill: parent
                radius: theme.radiusControl
                color: theme.surfaceRaised
                border.width: input.activeFocus ? 2 : 1
                border.color: torrentDrop.containsDrag ? theme.accent
                            : input.activeFocus ? theme.accent
                            : input.hovered ? Qt.rgba(palette.text.r, palette.text.g,
                                                       palette.text.b, 0.22)
                            : theme.hairline
            }

            ScrollView {
                id: scroll

                objectName: "newTaskInputScroll"
                anchors.fill: parent
                anchors.margins: 1
                clip: true
                rightPadding: inputVerticalBar.implicitWidth
                bottomPadding: inputHorizontalBar.implicitHeight
                background: Rectangle { color: "transparent" }
                ScrollBar.vertical: KScrollBar {
                    id: inputVerticalBar
                    objectName: "newTaskInputVerticalBar"
                    parent: scroll
                    anchors.top: parent.top
                    anchors.right: parent.right
                    anchors.bottom: parent.bottom
                    visible: size < 1
                }
                ScrollBar.horizontal: KScrollBar {
                    id: inputHorizontalBar
                    objectName: "newTaskInputHorizontalBar"
                    parent: scroll
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.bottom: parent.bottom
                    // KDE 原生背景会画整条轨道线；这里只画椭圆滑块。
                    background: null
                    implicitHeight: 6
                    padding: 0
                    visible: size < 1

                    contentItem: Rectangle {
                        implicitWidth: 6
                        implicitHeight: 6
                        radius: 3
                        color: Qt.rgba(palette.windowText.r, palette.windowText.g,
                                       palette.windowText.b,
                                       inputHorizontalBar.pressed ? 0.20
                                       : inputHorizontalBar.hovered ? 0.15 : 0.10)
                        border.width: 1
                        border.color: Qt.rgba(palette.windowText.r, palette.windowText.g,
                                              palette.windowText.b,
                                              inputHorizontalBar.pressed ? 0.38
                                              : inputHorizontalBar.hovered ? 0.32 : 0.26)
                    }
                }

                TextArea {
                    id: input

                    objectName: "newTaskInput"
                    width: Math.max(scroll.availableWidth,
                                    contentWidth + leftPadding + rightPadding)
                    padding: 11
                    color: theme.textPrimary
                    placeholderText: qsTr("magnet:?xt=...\n40 位 infohash\nhttps://...")
                    placeholderTextColor: theme.textMuted
                    font.pixelSize: theme.fontBody
                    selectByMouse: true
                    readOnly: dialog.parsingMagnet
                    wrapMode: TextEdit.NoWrap
                    background: Rectangle { color: "transparent" }
                }
            }

            Rectangle {
                objectName: "newTaskTorrentDropOverlay"
                anchors.fill: parent
                anchors.margins: 1
                radius: theme.radiusControl
                visible: torrentDrop.containsDrag
                color: Qt.rgba(theme.accent.r, theme.accent.g, theme.accent.b, 0.10)

                Label {
                    anchors.centerIn: parent
                    text: qsTr("松开以解析 .torrent")
                    color: theme.accent
                    font.pixelSize: theme.fontBody
                    font.weight: Font.DemiBold
                }
            }

            DropArea {
                id: torrentDrop

                objectName: "newTaskTorrentDropArea"
                anchors.fill: parent
                keys: ["text/uri-list"]
                onEntered: function (drag) {
                    drag.accepted = drag.hasUrls
                                     && dialog.backend !== null
                                     && typeof dialog.backend.previewTorrentFiles === "function"
                }
                onDropped: function (drop) {
                    if (!drop.hasUrls)
                        return
                    drop.acceptProposedAction()
                    dialog.previewTorrentUrls(drop.urls)
                }
            }
        }

    }

    TaskFolderPickerDialog {
        id: folderPicker
        backend: dialog.backend
        onFolderChosen: function (folderId, pathText) {
            dialog.targetFolderId = folderId
            dialog.targetFolderPath = pathText
            dialog.pickerStartPath = folderPicker.selectedPath.slice()
        }
    }

    MagnetFilesDialog {
        id: magnetDialog
        backend: dialog.backend
        // 保存已交给后台，选择框收起时连新任务弹窗一起关掉。
        onSaved: dialog.close()
    }

    function countLinks(text) {
        var lines = String(text || "").split("\n")
        var count = 0
        for (var i = 0; i < lines.length; i++) {
            if (lines[i].trim().length > 0)
                count += 1
        }
        return count
    }

    /// 第一行无法识别的输入（1 起）；全部有效或输入为空时返回 0。
    function firstInvalidLine(text) {
        var lines = String(text || "").split("\n")
        for (var i = 0; i < lines.length; i++) {
            var line = lines[i].trim()
            if (line.length > 0 && !dialog.isTaskLink(line))
                return i + 1
        }
        return 0
    }

    /// 与 kichi-core 的 is_supported_task_url 保持同一口径：磁力 / 纯 infohash / ed2k / HTTP(S)。
    function isTaskLink(value) {
        var line = String(value || "").trim()
        if (dialog.isBareInfohash(line))
            return true
        var lower = line.toLowerCase()
        if (lower.startsWith("magnet:?")) {
            var parts = lower.substring(8).split("&")
            for (var i = 0; i < parts.length; i++) {
                if (parts[i].startsWith("xt=urn:bt"))
                    return true
            }
            return false
        }
        if (lower.startsWith("ed2k://"))
            return line.length > 7
        if (lower.startsWith("https://"))
            return dialog.hasHost(line.substring(8))
        if (lower.startsWith("http://"))
            return dialog.hasHost(line.substring(7))
        return false
    }

    function hasHost(rest) {
        var value = rest.trim()
        return value.length > 0 && value.charAt(0) !== "/"
    }

    /// 所有非空行都是磁力链接时返回链接数组；混有其他链接或无效行时返回空数组。
    function allMagnetLinks(text) {
        var lines = String(text || "").split("\n")
        var links = []
        for (var i = 0; i < lines.length; i++) {
            var line = lines[i].trim()
            if (line.length === 0)
                continue
            if (!dialog.isMagnetLink(line))
                return []
            links.push(line)
        }
        return links
    }

    function isMagnetLink(value) {
        var line = String(value || "").trim()
        if (dialog.isBareInfohash(line))
            return true
        var lower = line.toLowerCase()
        if (!lower.startsWith("magnet:?"))
            return false
        var parts = lower.substring(8).split("&")
        for (var i = 0; i < parts.length; i++) {
            if (parts[i].startsWith("xt=urn:bt"))
                return true
        }
        return false
    }

    /// 不带 magnet: 前缀的 BTIH：40 位十六进制或 32 位 base32。
    function isBareInfohash(value) {
        var line = String(value || "").trim()
        return /^[0-9a-fA-F]{40}$/.test(line) || /^[a-zA-Z2-7]{32}$/.test(line)
    }

    /// 清单里可秒传（有 GCID）的文件数。
    function availableMagnetCount() {
        const rows = dialog.backend ? dialog.backend.magnetFiles : null
        if (!rows || rows.length === undefined)
            return 0
        let count = 0
        for (let i = 0; i < rows.length; i++) {
            if (rows[i] && rows[i].available === true)
                count += 1
        }
        return count
    }

    /// 单条磁链先解析，其余输入直接提交云下载。
    function submit() {
        if (!dialog.backend || !dialog.canSubmit || dialog.parsingMagnet)
            return
        const links = dialog.magnetLinks
        if (links.length > 0) {
            dialog.parsingMagnet = true
            dialog.torrentNotice = links.length > 1
                    ? qsTr("正在解析 %1 条磁链……").arg(links.length)
                    : qsTr("正在解析磁链……")
            dialog.torrentNoticeError = false
            dialog.backend.previewMagnetFiles(links.join("\n"))
            return
        }
        dialog.submitTask()
    }

    /// 绕过磁链解析，直接创建普通云下载任务。
    function submitTask() {
        if (!dialog.backend || !dialog.canSubmit)
            return
        dialog.submitText(input.text.trim())
        dialog.close()
    }

    /// 把一批链接交给普通云下载；返回是否真的提交了内容。
    function submitText(text) {
        const value = String(text || "").trim()
        if (!dialog.backend || value.length === 0)
            return false
        if (dialog.saveToFolder)
            dialog.backend.addTaskToFolder(value, String(dialog.targetFolderId))
        else
            dialog.backend.addTask(value)
        return true
    }

    function linksText(links) {
        if (!links || links.length === undefined)
            return ""
        const values = []
        for (let i = 0; i < links.length; i++) {
            const value = String(links[i] || "").trim()
            if (value.length > 0)
                values.push(value)
        }
        return values.join("\n")
    }

    function previewTorrentUrls(urls) {
        if (!dialog.backend || typeof dialog.backend.previewTorrentFiles !== "function"
                || !urls || urls.length === 0)
            return
        var accepted = []
        for (var i = 0; i < urls.length; i++) {
            var value = String(urls[i] || "")
            if (value.length > 0)
                accepted.push(value)
        }
        if (accepted.length === 0)
            return
        dialog.torrentNotice = qsTr("正在解析 .torrent……")
        dialog.torrentNoticeError = false
        dialog.backend.previewTorrentFiles(accepted)
    }

    function chooseTorrentFiles() {
        if (!dialog.backend || typeof dialog.backend.chooseTorrentFiles !== "function")
            return
        // 系统选择器本身负责选择中的反馈；取消时不留下永久提示。
        dialog.torrentNotice = ""
        dialog.torrentNoticeError = false
        dialog.backend.chooseTorrentFiles()
    }

    function appendTorrent(name, magnet, detail) {
        var value = String(magnet || "").trim()
        if (value.length === 0)
            return
        var current = String(input.text || "").trim()
        var lines = current.length > 0 ? current.split("\n") : []
        var present = false
        for (var i = 0; i < lines.length; i++) {
            if (String(lines[i]).trim() === value) {
                present = true
                break
            }
        }
        if (!present)
            input.text = current.length > 0 ? current + "\n" + value : value
        var title = String(name || "").trim()
        var extra = String(detail || "").trim()
        dialog.torrentNotice = extra.length > 0
                ? qsTr("已解析「%1」：%2，已加入输入框。").arg(title).arg(extra)
                : qsTr("已解析「%1」，已加入输入框。").arg(title)
        dialog.torrentNoticeError = false
        input.cursorPosition = input.text.length
    }

    Connections {
        target: dialog.backend
        ignoreUnknownSignals: true

        function onTorrentParsed(name, magnet, detail) {
            if (dialog.opened)
                dialog.appendTorrent(name, magnet, detail)
        }

        function onTorrentParseFailed(message) {
            if (!dialog.opened)
                return
            dialog.torrentNotice = String(message || qsTr("无法解析 .torrent 文件"))
            dialog.torrentNoticeError = true
        }

        function onMagnetPreviewFinished(ok, message, canFallback, fallbackLinks) {
            if (!dialog.opened || !dialog.parsingMagnet)
                return
            dialog.parsingMagnet = false

            if (!ok && !canFallback) {
                dialog.torrentNotice = String(message || qsTr("无法解析磁链"))
                dialog.torrentNoticeError = true
                return
            }

            // 解析失败或没有缓存的磁链单独回落普通云下载，其余进入同一个选择框。
            const fallbackText = dialog.linksText(fallbackLinks)
            const fallbackSubmitted = dialog.submitText(fallbackText)
            if (!ok || dialog.availableMagnetCount() === 0) {
                dialog.torrentNotice = !ok
                        ? qsTr("磁链暂时无法秒传，已改为普通云下载。")
                        : qsTr("云端暂无缓存，已改为普通云下载。")
                dialog.torrentNoticeError = false
                if (fallbackSubmitted)
                    dialog.close()
                else
                    dialog.submitTask()
                return
            }

            dialog.torrentNotice = ""
            dialog.torrentNoticeError = false
            // 官方 My Pack 只是一个下载位置，没有目录 id；磁链秒传必须落到根目录。
            magnetDialog.openFor(
                dialog.targetFolderId === null ? "" : String(dialog.targetFolderId),
                dialog.targetFolderId === null ? qsTr("全部文件") : dialog.targetFolderPath,
                dialog.pickerStartPath)
        }
    }
}
