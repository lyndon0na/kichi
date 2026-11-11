import QtQuick
import QtQuick.Controls

// 文件页右键「排序」子菜单：排序依据 + 顺序。
//
// 排序只是展示策略：调 backend.setSorting 后 Rust 对已缓存条目本地重排，
// 不重新拉网。列表表头点列走同一个入口（FileListHeader），两处结果必须一致。
//
// 打勾状态在每次打开时从后端**重读**，不写绑定：MenuItem 的 checkable 没有互斥
// 组，点一下就把 checked 换成命令值并永久断掉绑定（同一类坑），之后从表头改排序这里就同步不回来了。
Menu {
    id: menu

    property var backend: null

    title: qsTr("排序")
    // 宽度只写在菜单上
    width: 150
    enabled: menu.backend !== null

    // 换排序列时用该列的常用方向：名称从 A 到 Z，大小和时间把大的 / 新的放前面。
    // 与 FileListHeader.defaultAscending、SearchFilterButton.defaultAscending
    // 是同一条规则，三处要一起改。
    function defaultAscending(field) {
        return field === "name"
    }

    function applyField(field) {
        if (!menu.backend)
            return
        menu.backend.setSorting(field, menu.defaultAscending(field))
        menu.syncMarks()
    }

    function applyDirection(ascending) {
        if (!menu.backend)
            return
        menu.backend.setSorting(menu.backend.sortField, ascending)
        menu.syncMarks()
    }

    function syncMarks() {
        if (!menu.backend)
            return
        var field = menu.backend.sortField
        var ascending = menu.backend.sortAscending
        nameItem.checked = field === "name"
        sizeItem.checked = field === "size"
        modifiedItem.checked = field === "modified"
        ascendingItem.checked = ascending
        descendingItem.checked = !ascending
    }

    // 子菜单自己弹出时才同步（挂在空白菜单里时它还不是可见状态）
    onAboutToShow: menu.syncMarks()

    MenuItem {
        id: nameItem
        objectName: "sortByNameItem"
        text: qsTr("名称")
        checkable: true
        onTriggered: menu.applyField("name")
    }

    MenuItem {
        id: sizeItem
        objectName: "sortBySizeItem"
        text: qsTr("大小")
        checkable: true
        onTriggered: menu.applyField("size")
    }

    MenuItem {
        id: modifiedItem
        objectName: "sortByModifiedItem"
        text: qsTr("修改时间")
        checkable: true
        onTriggered: menu.applyField("modified")
    }

    MenuSeparator {}

    MenuItem {
        id: ascendingItem
        objectName: "sortAscendingItem"
        text: qsTr("升序")
        checkable: true
        onTriggered: menu.applyDirection(true)
    }

    MenuItem {
        id: descendingItem
        objectName: "sortDescendingItem"
        text: qsTr("降序")
        checkable: true
        onTriggered: menu.applyDirection(false)
    }
}
