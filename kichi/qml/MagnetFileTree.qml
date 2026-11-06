pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// 磁链文件树：按文件夹折叠，勾选文件夹会带上其下所有可秒传文件。
//
// 数据来自 backend 的 `magnetFiles`（每个文件带 `folders` 层级链）。这里把
// 文件组织成树，再按展开状态压平成 ListModel：折叠只改 `shown`，行号不变，
// 勾选不会因为折叠而丢失。
Item {
    id: tree

    property var rows: []
    property bool saving: false

    property int checkedCount: 0
    property int availableCount: 0
    property int totalFiles: 0
    readonly property bool allChecked: availableCount > 0 && checkedCount === availableCount
    readonly property int rowCount: nodes.count

    /// node.key → 树节点；node.key → ListModel 行号。只在 load() / 勾选时更新。
    property var nodesByKey: ({})
    property var rowByKey: ({})

    implicitWidth: 400
    implicitHeight: 300

    readonly property KTheme theme: KTheme { parent: tree }

    ListModel { id: nodes }

    ListView {
        id: fileList

        objectName: "magnetFilesList"
        anchors.fill: parent
        clip: true
        model: nodes
        boundsBehavior: Flickable.StopAtBounds
        ScrollBar.vertical: KScrollBar {}

        delegate: MagnetFileRow {
            saving: tree.saving
            onToggleRequested: function (rowIndex) {
                tree.toggleRow(rowIndex)
            }
            onExpandRequested: function (rowIndex) {
                tree.toggleExpanded(rowIndex)
            }
        }
    }

    Label {
        objectName: "magnetFilesEmpty"
        anchors.centerIn: parent
        visible: nodes.count === 0
        text: qsTr("这个磁链没有列出可保存的文件。")
        color: theme.textMuted
        font.pixelSize: theme.fontMeta
    }

    function clear() {
        nodes.clear()
        tree.nodesByKey = ({})
        tree.rowByKey = ({})
        tree.checkedCount = 0
        tree.availableCount = 0
        tree.totalFiles = 0
    }

    function load() {
        tree.clear()
        const roots = []
        const rootMap = Object.create(null)
        const source = tree.isArrayLike(tree.rows) ? tree.rows : []
        for (let i = 0; i < source.length; i++) {
            const raw = source[i] || {}
            const folders = folderNames(raw)
            let siblings = roots
            let byName = rootMap
            let parent = null
            let depth = 0
            for (let f = 0; f < folders.length; f++) {
                const folderName = folders[f]
                let folder = byName[folderName]
                if (!folder) {
                    folder = {
                        kind: "folder",
                        key: "d:" + (parent ? parent.key + "\u001f" : "") + folderName,
                        parent: parent,
                        depth: depth,
                        name: folderName,
                        children: [],
                        byName: Object.create(null),
                        expanded: true,
                        availableCount: 0,
                        checkedCount: 0
                    }
                    byName[folderName] = folder
                    siblings.push(folder)
                    tree.nodesByKey[folder.key] = folder
                }
                siblings = folder.children
                byName = folder.byName
                parent = folder
                depth += 1
            }

            const fileIndex = Number(raw.index !== undefined && raw.index !== null ? raw.index : i)
            const available = raw.available === true
            const file = {
                kind: "file",
                key: "f:" + fileIndex,
                parent: parent,
                depth: depth,
                name: String(raw.name || ""),
                size: String(raw.size || ""),
                note: raw.note ? String(raw.note) : "",
                available: available,
                selected: available,
                fileIndex: fileIndex
            }
            siblings.push(file)
            tree.nodesByKey[file.key] = file
            tree.totalFiles += 1
            if (available) {
                tree.availableCount += 1
                tree.checkedCount += 1
                let folder = parent
                while (folder) {
                    folder.availableCount += 1
                    folder.checkedCount += 1
                    folder = folder.parent
                }
            }
        }

        tree.flatten(roots, true)
        tree.recount()
    }

    /// cxx-qt 的 `QVariantList` 进 QML 后是带 `length` 的数组型对象，
    /// 不一定是真正的 JS Array——`Array.isArray` 会把它判成 false。
    function isArrayLike(value) {
        return value !== null && value !== undefined
                && typeof value.length === "number"
    }

    /// 文件夹层级链：优先用 backend 给的数组，老数据退回 `path` 字符串。
    function folderNames(raw) {
        let folders = []
        if (tree.isArrayLike(raw.folders)) {
            folders = raw.folders
        } else if (String(raw.path || "").length > 0) {
            folders = String(raw.path).split(" / ")
        }
        const names = []
        for (let i = 0; i < folders.length; i++) {
            const name = String(folders[i] || "")
            if (name.length > 0)
                names.push(name)
        }
        return names
    }

    function flatten(list, shown) {
        for (let i = 0; i < list.length; i++) {
            const node = list[i]
            tree.rowByKey[node.key] = nodes.count
            if (node.kind === "folder") {
                nodes.append({
                    isFolder: true,
                    nodeKey: node.key,
                    parentKey: node.parent ? node.parent.key : "",
                    depth: node.depth,
                    nodeName: node.name,
                    nodeSize: "",
                    nodeNote: "",
                    available: node.availableCount > 0,
                    selected: false,
                    nodeState: folderState(node),
                    expanded: node.expanded,
                    shown: shown,
                    fileIndex: -1
                })
                tree.flatten(node.children, shown && node.expanded)
            } else {
                nodes.append({
                    isFolder: false,
                    nodeKey: node.key,
                    parentKey: node.parent ? node.parent.key : "",
                    depth: node.depth,
                    nodeName: node.name,
                    nodeSize: node.size,
                    nodeNote: node.note,
                    available: node.available,
                    selected: node.selected,
                    nodeState: node.selected ? Qt.Checked : Qt.Unchecked,
                    expanded: false,
                    shown: shown,
                    fileIndex: node.fileIndex
                })
            }
        }
    }

    function folderState(node) {
        if (node.availableCount === 0 || node.checkedCount === 0)
            return Qt.Unchecked
        if (node.checkedCount === node.availableCount)
            return Qt.Checked
        return Qt.PartiallyChecked
    }

    function toggleExpanded(rowIndex) {
        const row = nodes.get(rowIndex)
        if (!row || !row.isFolder)
            return
        const node = tree.nodesByKey[row.nodeKey]
        if (!node)
            return
        node.expanded = !node.expanded
        nodes.setProperty(rowIndex, "expanded", node.expanded)
        tree.applyVisibility()
    }

    function toggleRow(rowIndex) {
        const row = nodes.get(rowIndex)
        if (!row)
            return
        const node = tree.nodesByKey[row.nodeKey]
        if (!node)
            return
        if (row.isFolder) {
            if (node.availableCount === 0)
                return
            tree.applySubtree(node, node.checkedCount < node.availableCount)
        } else {
            if (!node.available)
                return
            tree.applyFile(node, !node.selected)
        }
        tree.recount()
    }

    function applySubtree(node, value) {
        if (node.kind === "file") {
            tree.applyFile(node, value)
            return
        }
        for (let i = 0; i < node.children.length; i++)
            tree.applySubtree(node.children[i], value)
    }

    function applyFile(node, value) {
        if (!node.available || node.selected === value)
            return
        node.selected = value
        const rowIndex = tree.rowByKey[node.key]
        if (rowIndex !== undefined && rowIndex >= 0) {
            nodes.setProperty(rowIndex, "selected", value)
            nodes.setProperty(rowIndex, "nodeState", value ? Qt.Checked : Qt.Unchecked)
        }
        let folder = node.parent
        while (folder) {
            folder.checkedCount += value ? 1 : -1
            tree.updateFolderRow(folder)
            folder = folder.parent
        }
    }

    function updateFolderRow(node) {
        const rowIndex = tree.rowByKey[node.key]
        if (rowIndex === undefined || rowIndex < 0)
            return
        nodes.setProperty(rowIndex, "nodeState", folderState(node))
    }

    function applyVisibility() {
        const shownByKey = Object.create(null)
        for (let i = 0; i < nodes.count; i++) {
            const row = nodes.get(i)
            if (row.parentKey.length === 0) {
                shownByKey[row.nodeKey] = true
                continue
            }
            const parent = tree.nodesByKey[row.parentKey]
            const shown = shownByKey[row.parentKey] === true
                          && parent !== undefined && parent.expanded
            shownByKey[row.nodeKey] = shown
            if (row.shown !== shown)
                nodes.setProperty(i, "shown", shown)
        }
    }

    function setAllChecked(value) {
        for (let i = 0; i < nodes.count; i++) {
            const row = nodes.get(i)
            if (row.isFolder)
                continue
            const node = tree.nodesByKey[row.nodeKey]
            if (!node || !node.available)
                continue
            node.selected = value
            nodes.setProperty(i, "selected", value)
            nodes.setProperty(i, "nodeState", value ? Qt.Checked : Qt.Unchecked)
        }
        for (let i = 0; i < nodes.count; i++) {
            const row = nodes.get(i)
            if (!row.isFolder)
                continue
            const node = tree.nodesByKey[row.nodeKey]
            if (node) {
                node.checkedCount = value ? node.availableCount : 0
                nodes.setProperty(i, "nodeState", folderState(node))
            }
        }
        tree.recount()
    }

    function recount() {
        let checked = 0
        let available = 0
        let total = 0
        for (let i = 0; i < nodes.count; i++) {
            const row = nodes.get(i)
            if (row.isFolder)
                continue
            total += 1
            if (!row.available)
                continue
            available += 1
            if (row.selected)
                checked += 1
        }
        tree.totalFiles = total
        tree.availableCount = available
        tree.checkedCount = checked
    }

    function selectedIndices() {
        const indices = []
        for (let i = 0; i < nodes.count; i++) {
            const row = nodes.get(i)
            if (!row.isFolder && row.available && row.selected)
                indices.push(row.fileIndex)
        }
        return indices
    }
}
