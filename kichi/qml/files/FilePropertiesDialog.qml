pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "../components"
import "../core"

// 文件 / 文件夹属性弹窗（只读）。
//
// 打开时先用列表条目已有的字段立即显示，同时在后台按 id 请求一次详情
// （文件夹也支持详情）；详情回来后补齐缺失字段与完整路径。
// 读取失败只在弹窗里显示原因，文件页照常可用。
// 只展示接口真实给到的字段：文件夹大小固定「—」，路径拼不出来也显示「—」。
Item {
    id: root

    property var backend: null

    KTheme { id: theme }
    ClipboardHelper { id: clipboard }

    Timer {
        id: copyNoticeTimer
        interval: 1800
        onTriggered: dialog.copyNotice = ""
    }

    KDialog {
        id: dialog
        objectName: "propertiesDialog"

        readonly property int formWidth: 460

        title: qsTr("属性")
        modal: true
        parent: Overlay.overlay
        anchors.centerIn: Overlay.overlay
        width: formWidth + leftPadding + rightPadding
        spacing: theme.gapMd

        // 当前请求对应的条目 id；异步结果回来时只认这一个
        property string lookupId: ""
        property string nameText: "—"
        property string kindText: "—"
        property string sizeText: "—"
        property string createdText: "—"
        property string modifiedText: "—"
        property string entryIdText: "—"
        property string pathText: "—"
        property bool loading: false
        property string errorText: ""
        property string copyNotice: ""

        footer: DialogFooter {
            outerMargin: dialog.padding
            okText: qsTr("关闭")
            okVariant: "secondary"
            cancelVisible: false
            statusText: dialog.copyNotice
            statusColor: theme.success
            onOkClicked: dialog.close()
        }

        ColumnLayout {
            width: dialog.formWidth
            spacing: 10

            GridLayout {
                Layout.fillWidth: true
                columns: 2
                columnSpacing: theme.gapLg
                rowSpacing: 9

                Label {
                    text: qsTr("名称")
                    color: theme.textSecondary
                    font.pixelSize: theme.fontMeta
                    Layout.alignment: Qt.AlignTop
                }
                ElidedLabel {
                    objectName: "propertyName"
                    Layout.fillWidth: true
                    text: dialog.nameText
                    color: theme.textPrimary
                    font.pixelSize: theme.fontBody
                    elide: Text.ElideMiddle
                }

                Label {
                    text: qsTr("类型")
                    color: theme.textSecondary
                    font.pixelSize: theme.fontMeta
                }
                ElidedLabel {
                    objectName: "propertyType"
                    Layout.fillWidth: true
                    text: dialog.kindText
                    color: theme.textPrimary
                    font.pixelSize: theme.fontBody
                }

                Label {
                    text: qsTr("大小")
                    color: theme.textSecondary
                    font.pixelSize: theme.fontMeta
                }
                ElidedLabel {
                    objectName: "propertySize"
                    Layout.fillWidth: true
                    text: dialog.sizeText
                    color: theme.textPrimary
                    font.pixelSize: theme.fontBody
                }

                Label {
                    text: qsTr("创建时间")
                    color: theme.textSecondary
                    font.pixelSize: theme.fontMeta
                }
                ElidedLabel {
                    objectName: "propertyCreated"
                    Layout.fillWidth: true
                    text: dialog.createdText
                    color: theme.textPrimary
                    font.pixelSize: theme.fontBody
                }

                Label {
                    text: qsTr("修改时间")
                    color: theme.textSecondary
                    font.pixelSize: theme.fontMeta
                }
                ElidedLabel {
                    objectName: "propertyModified"
                    Layout.fillWidth: true
                    text: dialog.modifiedText
                    color: theme.textPrimary
                    font.pixelSize: theme.fontBody
                }

                Label {
                    text: qsTr("云端条目 ID")
                    color: theme.textSecondary
                    font.pixelSize: theme.fontMeta
                }
                RowLayout {
                    Layout.fillWidth: true
                    spacing: theme.gapSm

                    ElidedLabel {
                        objectName: "propertyId"
                        Layout.fillWidth: true
                        text: dialog.entryIdText
                        color: theme.textPrimary
                        font.pixelSize: theme.fontMeta
                        elide: Text.ElideMiddle
                    }

                    // 只给云端 id 提供复制入口：时间 / 大小没有转发价值，
                    // 名称和路径在列表与面包屑里本来就能选中复制。
                    KButton {
                        objectName: "propertiesCopyId"
                        text: qsTr("复制")
                        variant: "ghost"
                        implicitHeight: 28
                        leftPadding: 10
                        rightPadding: 10
                        enabled: dialog.entryIdText.length > 0 && dialog.entryIdText !== "—"
                        onClicked: root.copyEntryId()
                    }
                }

                Label {
                    text: qsTr("当前所在路径")
                    color: theme.textSecondary
                    font.pixelSize: theme.fontMeta
                    Layout.alignment: Qt.AlignTop
                }
                ElidedLabel {
                    objectName: "propertyPath"
                    Layout.fillWidth: true
                    text: dialog.pathText
                    color: theme.textPrimary
                    font.pixelSize: theme.fontBody
                    elide: Text.ElideMiddle
                }
            }

            Label {
                objectName: "propertiesLoading"
                Layout.fillWidth: true
                visible: dialog.loading
                text: qsTr("正在读取详细信息…")
                color: theme.textMuted
                font.pixelSize: theme.fontMeta
            }

            Label {
                objectName: "propertiesError"
                Layout.fillWidth: true
                visible: dialog.errorText.length > 0
                text: dialog.errorText
                color: theme.danger
                font.pixelSize: theme.fontMeta
                wrapMode: Text.Wrap
            }
        }
    }

    /// `fromSearch` 为真时不拿文件页面包屑当路径（其它页的条目不一定在当前目录）；
    /// `pathHint` 是调用页已知的真实路径。
    function openFor(entry, fromSearch, pathHint) {
        if (!entry || !root.backend)
            return
        const id = String(entry.id || "")
        const name = String(entry.name || "")
        const folder = entry.kind === "folder"
        dialog.lookupId = id
        dialog.nameText = name.length > 0 ? name : "—"
        // 离线任务的产物可能只给 id 不给类型：交给详情接口判断，先显示「—」
        dialog.kindText = folder
                ? qsTr("文件夹")
                : (entry.kind === "file" ? qsTr("文件") : "—")
        dialog.sizeText = folder ? "—" : textOrDash(entry.sizeText)
        dialog.createdText = textOrDash(entry.createdText)
        dialog.modifiedText = textOrDash(entry.modifiedText)
        dialog.entryIdText = id.length > 0 ? id : "—"
        const hint = String(pathHint || "").trim()
        dialog.pathText = hint.length > 0
                ? hint
                : textOrDash(root.pathHint(entry, fromSearch === true))
        // 只在确实缺字段（或路径未知）时请求详情，已有完整数据的条目不必多发请求。
        dialog.loading = id.length > 0
                && (dialog.createdText === "—" || dialog.modifiedText === "—"
                    || dialog.pathText === "—")
        dialog.errorText = ""
        dialog.copyNotice = ""
        dialog.open()
        if (dialog.loading
                && typeof root.backend.requestEntryProperties === "function")
            root.backend.requestEntryProperties(id)
    }

    /// 初始路径提示：只有当前目录里的条目能直接拼准。
    /// 搜索结果的父目录可能在网盘任意位置，交给后端按 id 还原；
    /// 拼不出来时显示「—」，不拿 id 冒充路径。
    function pathHint(entry, fromSearch) {
        if (!entry || !root.backend || fromSearch)
            return ""
        const crumbs = root.backend.breadcrumb ? root.backend.breadcrumb : []
        const parts = []
        for (let i = 0; i < crumbs.length; i++)
            parts.push(String(crumbs[i]))
        if (parts.length === 0)
            parts.push(qsTr("全部文件"))
        parts.push(String(entry.name || ""))
        return parts.join(" / ")
    }

    function textOrDash(value) {
        const text = String(value || "").trim()
        return text.length > 0 ? text : "—"
    }

    function copyEntryId() {
        if (dialog.entryIdText.length === 0 || dialog.entryIdText === "—")
            return
        if (!clipboard.copyText(dialog.entryIdText))
            return
        dialog.copyNotice = qsTr("云端条目 ID 已复制")
        copyNoticeTimer.restart()
    }

    Connections {
        target: root.backend
        ignoreUnknownSignals: true

        function onEntryPropertiesReady(id, ok, message, name, kind, size, created,
                                        modified, entryId, path) {
            if (!dialog.visible || String(id) !== dialog.lookupId)
                return
            dialog.loading = false
            if (!ok) {
                const reason = String(message || "").trim()
                dialog.errorText = reason.length > 0
                        ? qsTr("读取详细信息失败：%1").arg(reason)
                        : qsTr("读取详细信息失败")
                return
            }
            dialog.errorText = ""
            dialog.nameText = textOrDash(name)
            dialog.kindText = textOrDash(kind)
            dialog.sizeText = textOrDash(size)
            dialog.createdText = textOrDash(created)
            dialog.modifiedText = textOrDash(modified)
            dialog.entryIdText = textOrDash(entryId)
            // 路径拼不出来时后端给空串：保留列表上下文里的路径提示，不拿 id 冒充
            const location = String(path || "").trim()
            if (location.length > 0)
                dialog.pathText = location
        }
    }
}
